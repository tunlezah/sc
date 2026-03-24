import { useState } from 'preact/hooks';
import type { DeviceInfo, DeviceState } from '../../types';
import * as api from '../../api/rest';

interface DeviceListProps {
  devices: DeviceInfo[];
  activeDevice: string | null;
  status: string;
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

function codecLabel(codec: string | null): string {
  if (!codec) return '';
  const labels: Record<string, string> = {
    sbc: 'SBC', aac: 'AAC', ldac: 'LDAC', apt_x: 'aptX', apt_x_hd: 'aptX HD',
  };
  return labels[codec] || codec;
}

export function DeviceList({ devices, activeDevice: _activeDevice, status }: DeviceListProps) {
  const [collapsed, setCollapsed] = useState(false);

  const handleScan = () => {
    if (status === 'scanning') {
      api.stopScan();
    } else {
      api.startScan();
    }
  };

  return (
    <div class="card" style={{ flex: '1 1 50%', minHeight: 0 }}>
      <div class="card-header" onClick={() => setCollapsed(!collapsed)}>
        <span class="card-title">
          Bluetooth Devices ({devices.length})
          {collapsed ? ' +' : ' -'}
        </span>
        <button class="btn btn-sm btn-primary" onClick={(e) => { e.stopPropagation(); handleScan(); }}>
          {status === 'scanning' ? 'Stop Scan' : 'Scan'}
        </button>
      </div>
      <div class={`card-content ${collapsed ? 'collapsed' : ''}`}>
        {devices.length === 0 ? (
          <div class="empty-state">No devices found. Start scanning to discover Bluetooth devices.</div>
        ) : (
          <div class="device-list">
            {devices.map((device) => (
              <div class="device-item" key={device.address}>
                <div class="device-info">
                  <div class="device-name">{device.name || device.address}</div>
                  <div class="device-details">
                    <span>{device.address}</span>
                    {device.rssi !== null && <span>{device.rssi} dBm</span>}
                    {device.codec && <span>{codecLabel(device.codec)}</span>}
                    {stateBadge(device.state)}
                  </div>
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
    </div>
  );
}
