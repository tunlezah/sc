import { useState } from 'preact/hooks';
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

  const filtered = filter
    ? devices.filter((d) => {
        const q = filter.toLowerCase();
        return (d.name || '').toLowerCase().includes(q) || d.address.toLowerCase().includes(q);
      })
    : devices;

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
            Bluetooth ({devices.length})
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

            {devices.length > 0 && (
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
                  : 'No devices match your search.'}
              </div>
            ) : (
              <div class="device-list">
                {filtered.map((device) => (
                  <div class="device-item" key={device.address}>
                    <div class="device-info">
                      <div class="device-name">{device.name || device.address}</div>
                      <div class="device-details">
                        <span>{device.address}</span>
                        {device.rssi !== null && <span>{device.rssi} dBm</span>}
                        {stateBadge(device.state)}
                      </div>
                      {device.codec && <CodecStrip activeCodec={device.codec} />}
                    </div>
                    <div class="device-actions">
                      {(device.state === 'discovered' || device.state === 'paired' || device.state === 'disconnected') && (
                        <button class="btn btn-sm btn-primary" onClick={() => api.connectDevice(device.address)}>Connect</button>
                      )}
                      {(device.state === 'connected' || device.state === 'audio_active' || device.state === 'profile_negotiated' || device.state === 'pipewire_source_ready') && (
                        <button class="btn btn-sm btn-secondary" onClick={() => api.disconnectDevice(device.address)}>Disconnect</button>
                      )}
                      <button class="btn btn-sm btn-danger" onClick={() => api.removeDevice(device.address)}>Remove</button>
                    </div>
                  </div>
                ))}
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
