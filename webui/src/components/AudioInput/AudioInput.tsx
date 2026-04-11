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

  useEffect(() => {
    localStorage.setItem(SHOW_BLE_KEY, showBle ? 'true' : 'false');
  }, [showBle]);

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

  const handleLineInToggle = async () => {
    if (lineInActive) {
      await api.deactivateLineIn();
    } else {
      await api.activateLineIn();
    }
  };

  // BLE filter first (hide noise), then text search.
  const visible = showBle ? devices : devices.filter((d) => d.type !== 'ble');
  const filtered = filter
    ? visible.filter((d) => {
        const q = filter.toLowerCase();
        return (d.name || '').toLowerCase().includes(q) || d.address.toLowerCase().includes(q);
      })
    : visible;

  const bleHiddenCount = showBle ? 0 : devices.length - visible.length;

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
            Bluetooth ({visible.length}{bleHiddenCount > 0 ? ` / ${devices.length}` : ''})
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
              {status.startsWith('error') && (
                <span class="badge badge-disconnected" style={{ marginLeft: '8px', fontSize: '0.7rem' }}>
                  {status.replace('error:', '')}
                </span>
              )}
            </div>

            <div class="device-controls">
              <label class="device-ble-toggle" title="Show Bluetooth Low Energy devices in the list">
                <button
                  type="button"
                  class={`toggle toggle-sm ${showBle ? 'active' : ''}`}
                  aria-pressed={showBle}
                  onClick={() => setShowBle(!showBle)}
                />
                <span>Show BLE Devices</span>
              </label>
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
                    ? `BLE devices hidden (${bleHiddenCount}). Toggle "Show BLE Devices" to see them.`
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
