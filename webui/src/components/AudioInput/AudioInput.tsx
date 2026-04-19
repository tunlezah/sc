import { useState, useEffect } from 'preact/hooks';
import type { DeviceInfo, DeviceState } from '../../types';
import * as api from '../../api/rest';
import { CodecStrip } from '../DeviceList/CodecStrip';
import '../DeviceList/CodecStrip.css';

type InputTab = 'bluetooth' | 'line_in';

interface AudioInputProps {
  devices: DeviceInfo[];
  activeDevice: string | null;
  status: string;
  lineInActive: boolean;
  lineInAvailable: boolean;
}

const SHOW_BLE_KEY = 'soundsync-show-ble';
const HIDE_UNNAMED_KEY = 'soundsync-hide-unnamed';

/// Audio-capable device — stays visible even if BlueZ hasn't resolved a
/// friendly name yet, because the user likely wants to see it.
function isAudioCandidate(d: DeviceInfo): boolean {
  return d.has_a2dp || d.is_a2dp_source;
}

/// A device is "identifiable" if the user can tell what it is: either it
/// has a friendly name, or it advertises an audio profile (named "Audio
/// Source" / "Audio" in the UI). Bare MAC-only non-audio devices are the
/// noise we filter out.
function isIdentifiable(d: DeviceInfo): boolean {
  return d.name.trim() !== '' || isAudioCandidate(d);
}

function stateBadge(state: DeviceState) {
  const labels: Record<DeviceState, { label: string; cls: string }> = {
    disconnected: { label: 'Offline', cls: 'badge-disconnected' },
    discovered: { label: 'Found', cls: 'badge-discovered' },
    pairing: { label: 'Pairing', cls: 'badge-discovered' },
    paired: { label: 'Paired', cls: 'badge-discovered' },
    connected: { label: 'Connected', cls: 'badge-connected' },
    profile_negotiated: { label: 'Connected', cls: 'badge-connected' },
    pipewire_source_ready: { label: 'Ready', cls: 'badge-connected' },
    audio_active: { label: 'Streaming', cls: 'badge-audio' },
  };
  const { label, cls } = labels[state] || { label: state, cls: 'badge-disconnected' };
  return <span class={`badge ${cls}`}>{label}</span>;
}

export function AudioInput({ devices, activeDevice: _activeDevice, status, lineInActive, lineInAvailable }: AudioInputProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [activeTab, setActiveTab] = useState<InputTab>('bluetooth');
  const [filter, setFilter] = useState('');
  // Track which devices have a pending connect so we can show "Connecting..."
  const [connecting, setConnecting] = useState<Set<string>>(new Set());
  const [showBle, setShowBle] = useState<boolean>(() => {
    const stored = localStorage.getItem(SHOW_BLE_KEY);
    return stored === null ? true : stored !== 'false';
  });
  // Default ON for new users: the common complaint is a wall of MAC-only
  // mystery devices. Previously-saved preferences still win.
  const [hideUnnamed, setHideUnnamed] = useState<boolean>(() => {
    const stored = localStorage.getItem(HIDE_UNNAMED_KEY);
    return stored === null ? true : stored === 'true';
  });

  useEffect(() => {
    localStorage.setItem(SHOW_BLE_KEY, showBle ? 'true' : 'false');
  }, [showBle]);

  useEffect(() => {
    localStorage.setItem(HIDE_UNNAMED_KEY, hideUnnamed ? 'true' : 'false');
  }, [hideUnnamed]);

  // Clear "Connecting..." for devices that have progressed past discovered/paired/disconnected
  useEffect(() => {
    setConnecting((prev) => {
      const next = new Set(prev);
      for (const addr of prev) {
        const dev = devices.find((d) => d.address === addr);
        if (dev && dev.state !== 'discovered' && dev.state !== 'paired' && dev.state !== 'disconnected') {
          next.delete(addr);
        }
      }
      return next.size === prev.size ? prev : next;
    });
  }, [devices]);

  const handleScan = () => {
    if (status === 'scanning') {
      api.stopScan().catch(() => {});
    } else {
      api.startScan().catch((err) => {
        console.error('Scan request failed:', err);
      });
    }
  };

  // Active name-resolution: fires hcitool Remote-Name-Requests against every
  // device in the list that's still showing a MAC. Times out server-side
  // (~7s per device, 3 in parallel) so the spinner is a coarse progress hint.
  const [identifying, setIdentifying] = useState(false);
  const handleIdentify = () => {
    if (identifying) return;
    setIdentifying(true);
    api.resolveDeviceNames()
      .catch((err) => console.error('Identify request failed:', err))
      .finally(() => {
        // The server streams names back via websocket; give it a budget
        // matching the per-device timeout (7s) × bounded concurrency (3)
        // for a rough worst-case before letting the user click again.
        setTimeout(() => setIdentifying(false), 8000);
      });
  };

  const unnamedCount = devices.filter((d) => !d.name || d.name.trim() === '').length;

  const handleLineInToggle = async () => {
    if (lineInActive) {
      await api.deactivateLineIn();
    } else {
      await api.activateLineIn();
    }
  };

  // BLE filter first (hide noise), then identifiability filter, then text search.
  const afterBle = showBle ? devices : devices.filter((d) => d.type !== 'ble');
  const visible = hideUnnamed ? afterBle.filter(isIdentifiable) : afterBle;
  const filtered = filter
    ? visible.filter((d) => {
        const q = filter.toLowerCase();
        return (d.name || '').toLowerCase().includes(q) || d.address.toLowerCase().includes(q);
      })
    : visible;

  const bleHiddenCount = showBle ? 0 : devices.length - afterBle.length;
  const unnamedHiddenCount = hideUnnamed ? afterBle.length - visible.length : 0;
  const totalHiddenCount = bleHiddenCount + unnamedHiddenCount;

  const hasActiveBt = devices.some(d =>
    d.state === 'connected' || d.state === 'audio_active' ||
    d.state === 'profile_negotiated' || d.state === 'pipewire_source_ready'
  );

  return (
    <div class="card" style={{ flex: '1 1 50%', minHeight: 0 }}>
      <div class="card-header" onClick={() => setCollapsed(!collapsed)}>
        <span class="card-title">
          Audio Input
          {(hasActiveBt || lineInActive) && <span class="badge badge-audio" style={{ marginLeft: '6px' }}>Active</span>}
          {collapsed ? ' +' : ' -'}
        </span>
      </div>
      <div class={`card-content ${collapsed ? 'collapsed' : ''}`}>
        <div class="output-tabs">
          <button
            class={`output-tab ${activeTab === 'bluetooth' ? 'active' : ''}`}
            onClick={() => setActiveTab('bluetooth')}
          >
            Bluetooth ({visible.length}{totalHiddenCount > 0 ? ` / ${devices.length}` : ''})
          </button>
          <button
            class={`output-tab ${activeTab === 'line_in' ? 'active' : ''}`}
            onClick={() => setActiveTab('line_in')}
          >
            Line In {lineInActive && <span class="badge badge-audio" style={{ marginLeft: '4px', fontSize: '0.55rem' }}>Active</span>}
          </button>
        </div>

        {activeTab === 'bluetooth' && (
          <div class="output-panel">
            <div class="output-panel-header">
              <button class="btn btn-sm btn-primary" onClick={handleScan}>
                {status === 'scanning' ? 'Stop Scan' : 'Scan'}
              </button>
              <button
                class="btn btn-sm btn-secondary"
                style={{ marginLeft: '6px' }}
                onClick={handleIdentify}
                disabled={identifying || unnamedCount === 0}
                title="Issue an active HCI Remote-Name-Request against every device still showing a MAC address. Works on classic Bluetooth devices that never broadcast their name, without requiring pairing."
              >
                {identifying
                  ? 'Identifying...'
                  : unnamedCount > 0
                    ? `Identify (${unnamedCount})`
                    : 'Identify'}
              </button>
              {status.startsWith('error') && (
                <span class="badge badge-disconnected" style={{ marginLeft: '8px', fontSize: '0.7rem' }}>
                  {status.replace('error:', '')}
                </span>
              )}
            </div>

            <div class="device-controls">
              <div class="device-toggles">
                <label class="device-ble-toggle" title="Show Bluetooth Low Energy devices in the list">
                  <button
                    type="button"
                    class={`toggle toggle-sm ${showBle ? 'active' : ''}`}
                    aria-pressed={showBle}
                    onClick={() => setShowBle(!showBle)}
                  />
                  <span>Show BLE Devices</span>
                </label>
                <label
                  class="device-ble-toggle"
                  title="Hide devices with no friendly name and no detected audio profile. Devices advertising an audio profile stay visible even before BlueZ resolves their name."
                >
                  <button
                    type="button"
                    class={`toggle toggle-sm ${hideUnnamed ? 'active' : ''}`}
                    aria-pressed={hideUnnamed}
                    onClick={() => setHideUnnamed(!hideUnnamed)}
                  />
                  <span>Hide Unnamed Devices</span>
                </label>
              </div>
              <div class="device-legend" aria-hidden="true">
                <span class="device-legend-item">
                  <span class="device-legend-swatch device-legend-swatch--classic" />
                  Audio
                </span>
                <span class="device-legend-item">
                  <span class="device-legend-swatch device-legend-swatch--ble" />
                  BLE
                </span>
              </div>
            </div>

            {visible.length > 0 && (
              <div class="device-search">
                <input
                  type="text"
                  class="device-search-input"
                  placeholder="Search devices..."
                  value={filter}
                  onInput={(e) => setFilter((e.target as HTMLInputElement).value)}
                />
              </div>
            )}

            {filtered.length === 0 ? (
              <div class="empty-state">
                {devices.length === 0
                  ? 'No devices found. Start scanning to discover Bluetooth devices.'
                  : visible.length === 0
                    ? unnamedHiddenCount > 0 && bleHiddenCount === 0
                      ? `${unnamedHiddenCount} unnamed device${unnamedHiddenCount === 1 ? '' : 's'} hidden. Toggle "Hide Unnamed Devices" to see them, or wait a few seconds for names to resolve.`
                      : bleHiddenCount > 0 && unnamedHiddenCount === 0
                        ? `BLE devices hidden (${bleHiddenCount}). Toggle "Show BLE Devices" to see them.`
                        : `${totalHiddenCount} device${totalHiddenCount === 1 ? '' : 's'} hidden by filters. Toggle "Show BLE Devices" or "Hide Unnamed Devices" to see them.`
                    : 'No devices match your search.'}
              </div>
            ) : (
              <div class="device-list">
                {filtered.map((device) => {
                  const isBle = device.type === 'ble';
                  const itemCls = isBle ? 'device-item device-item--ble' : 'device-item device-item--classic';
                  const nameCls = isBle ? 'device-name device-name--ble' : 'device-name device-name--classic';
                  return (
                    <div class={itemCls} key={device.address}>
                      <div class="device-info">
                        <div class={nameCls}>
                          {device.name || device.address}
                          {device.is_a2dp_source && (
                            <span class="device-source-tag" title="Advertises A2DP audio source">Audio Source</span>
                          )}
                        </div>
                        <div class="device-details">
                          <span>{device.address}</span>
                          {device.rssi !== null && <span>{device.rssi} dBm</span>}
                          {stateBadge(device.state)}
                        </div>
                        {device.codec && <CodecStrip activeCodec={device.codec} />}
                      </div>
                      <div class="device-actions">
                        {(device.state === 'discovered' || device.state === 'paired' || device.state === 'disconnected') && (
                          <button
                            class="btn btn-sm btn-primary"
                            disabled={connecting.has(device.address)}
                            onClick={() => {
                              setConnecting((prev) => new Set(prev).add(device.address));
                              api.connectDevice(device.address).catch(() => {
                                setConnecting((prev) => { const n = new Set(prev); n.delete(device.address); return n; });
                              });
                            }}
                          >
                            {connecting.has(device.address) ? 'Connecting...' : 'Connect'}
                          </button>
                        )}
                        {(device.state === 'connected' || device.state === 'audio_active' || device.state === 'profile_negotiated' || device.state === 'pipewire_source_ready') && (
                          <button class="btn btn-sm btn-secondary" onClick={() => api.disconnectDevice(device.address)}>Disconnect</button>
                        )}
                        <button class="btn btn-sm btn-danger" onClick={() => api.removeDevice(device.address)}>Remove</button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {activeTab === 'line_in' && (
          <div class="output-panel">
            {lineInAvailable ? (
              <div class="output-line-in-status">
                <span class="output-line-in-label">
                  Line-In Source {lineInActive ? <span class="badge badge-audio">Active</span> : <span class="badge badge-disconnected">Inactive</span>}
                </span>
                <button
                  class={`btn btn-sm ${lineInActive ? 'btn-danger' : 'btn-primary'}`}
                  onClick={handleLineInToggle}
                >
                  {lineInActive ? 'Deactivate' : 'Activate'}
                </button>
              </div>
            ) : (
              <div class="empty-state">No line-in source detected.</div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
