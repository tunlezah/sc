import { useState } from 'preact/hooks';
import type { CastDevice, AirPlayDevice, DeviceInfo } from '../../types';
import * as api from '../../api/rest';

type OutputTab = 'bluetooth' | 'chromecast' | 'airplay';

interface AudioOutputProps {
  castDevices: CastDevice[];
  castActive: string | null;
  airplayDevices: AirPlayDevice[];
  airplayActive: string | null;
  devices: DeviceInfo[];
}

export function AudioOutput({
  castDevices,
  castActive,
  airplayDevices,
  airplayActive,
  devices,
}: AudioOutputProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [activeTab, setActiveTab] = useState<OutputTab>('bluetooth');
  const [castScanning, setCastScanning] = useState(false);
  const [airplayScanning, setAirplayScanning] = useState(false);
  const [castVolume, setCastVolume] = useState(80);
  const [airplayVolume, setAirplayVolume] = useState(80);

  const handleCastScan = async () => {
    setCastScanning(true);
    await api.castDiscover();
    setTimeout(() => setCastScanning(false), 12000);
  };

  const handleAirplayScan = async () => {
    setAirplayScanning(true);
    await api.airplayDiscover();
    setTimeout(() => setAirplayScanning(false), 12000);
  };

  const handleCastConnect = async (deviceId: string) => {
    await api.castConnect(deviceId);
  };

  const handleCastDisconnect = async () => {
    await api.castDisconnect();
  };

  const handleAirplayConnect = async (name: string) => {
    await api.airplayConnect(name);
  };

  const handleAirplayDisconnect = async () => {
    await api.airplayDisconnect();
  };

  const handleCastVolumeChange = async (value: number) => {
    setCastVolume(value);
    await api.castVolume(value / 100);
  };

  const handleAirplayVolumeChange = async (value: number) => {
    setAirplayVolume(value);
    await api.airplayVolume(value / 100);
  };

  const connectedBtDevices = devices.filter((d) =>
    ['connected', 'profile_negotiated', 'pipewire_source_ready', 'audio_active'].includes(d.state)
  );
  const btStreaming = devices.filter((d) => d.state === 'audio_active');
  const isActive = castActive !== null || airplayActive !== null || btStreaming.length > 0;

  return (
    <div class="card" style={{ flex: '1 1 0', minHeight: 0 }}>
      <div class="card-header" onClick={() => setCollapsed(!collapsed)}>
        <span class="card-title">
          Audio Output
          {isActive && <span class="badge badge-audio" style={{ marginLeft: '6px' }}>Active</span>}
          {collapsed ? ' +' : ' -'}
        </span>
      </div>
      <div class={`card-content ${collapsed ? 'collapsed' : ''}`}>
        <div class="output-tabs">
          <button
            class={`output-tab ${activeTab === 'bluetooth' ? 'active' : ''}`}
            onClick={() => setActiveTab('bluetooth')}
          >
            Bluetooth ({connectedBtDevices.length})
          </button>
          <button
            class={`output-tab ${activeTab === 'chromecast' ? 'active' : ''}`}
            onClick={() => setActiveTab('chromecast')}
          >
            Chromecast ({castDevices.length})
          </button>
          <button
            class={`output-tab ${activeTab === 'airplay' ? 'active' : ''}`}
            onClick={() => setActiveTab('airplay')}
          >
            AirPlay ({airplayDevices.length})
          </button>
        </div>

        {activeTab === 'bluetooth' && (
          <div class="output-panel">
            {connectedBtDevices.length === 0 ? (
              <div class="empty-state">No Bluetooth devices connected. Pair a device from the Bluetooth Devices section above.</div>
            ) : (
              <div class="device-list">
                {connectedBtDevices.map((device) => (
                  <div class="device-item" key={device.address}>
                    <div class="device-info">
                      <div class="device-name">{device.name || device.address}</div>
                      <div class="device-details">
                        <span>{device.address}</span>
                        {device.codec && <span>{device.codec.toUpperCase()}</span>}
                        {device.state === 'audio_active' ? (
                          <span class="badge badge-audio">Streaming</span>
                        ) : (
                          <span class="badge badge-connected">Connected</span>
                        )}
                      </div>
                    </div>
                    <div class="device-actions">
                      <button class="btn btn-sm btn-secondary" onClick={() => api.disconnectDevice(device.address)}>Disconnect</button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {activeTab === 'chromecast' && (
          <div class="output-panel">
            <div class="output-panel-header">
              <button
                class="btn btn-sm btn-primary"
                onClick={handleCastScan}
                disabled={castScanning}
              >
                {castScanning ? 'Scanning...' : 'Scan'}
              </button>
            </div>

            {castActive && (
              <div class="output-active-session">
                <div class="output-active-label">
                  Casting to: <strong>{castDevices.find(d => d.id === castActive)?.name || castActive}</strong>
                </div>
                <div class="output-volume-row">
                  <span class="output-volume-label">Vol</span>
                  <input
                    type="range"
                    min="0"
                    max="100"
                    value={castVolume}
                    class="output-volume-slider"
                    onInput={(e) => handleCastVolumeChange(parseInt((e.target as HTMLInputElement).value))}
                  />
                  <span class="output-volume-value">{castVolume}%</span>
                </div>
                <button class="btn btn-sm btn-danger" onClick={handleCastDisconnect}>Disconnect</button>
              </div>
            )}

            {castDevices.length === 0 ? (
              <div class="empty-state">No Chromecast devices found. Press Scan to discover.</div>
            ) : (
              <div class="device-list">
                {castDevices.map((device) => (
                  <div class="device-item" key={device.id}>
                    <div class="device-info">
                      <div class="device-name">{device.name}</div>
                      <div class="device-details">
                        <span>{device.model}</span>
                        {castActive === device.id && <span class="badge badge-audio">Casting</span>}
                      </div>
                    </div>
                    <div class="device-actions">
                      {castActive === device.id ? (
                        <button class="btn btn-sm btn-secondary" onClick={handleCastDisconnect}>Stop</button>
                      ) : (
                        <button
                          class="btn btn-sm btn-primary"
                          onClick={() => handleCastConnect(device.id)}
                          disabled={castActive !== null}
                        >Cast</button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {activeTab === 'airplay' && (
          <div class="output-panel">
            <div class="output-panel-header">
              <button
                class="btn btn-sm btn-primary"
                onClick={handleAirplayScan}
                disabled={airplayScanning}
              >
                {airplayScanning ? 'Scanning...' : 'Scan'}
              </button>
            </div>

            {airplayActive && (
              <div class="output-active-session">
                <div class="output-active-label">
                  Playing on: <strong>{airplayActive}</strong>
                </div>
                <div class="output-volume-row">
                  <span class="output-volume-label">Vol</span>
                  <input
                    type="range"
                    min="0"
                    max="100"
                    value={airplayVolume}
                    class="output-volume-slider"
                    onInput={(e) => handleAirplayVolumeChange(parseInt((e.target as HTMLInputElement).value))}
                  />
                  <span class="output-volume-value">{airplayVolume}%</span>
                </div>
                <button class="btn btn-sm btn-danger" onClick={handleAirplayDisconnect}>Disconnect</button>
              </div>
            )}

            {airplayDevices.length === 0 ? (
              <div class="empty-state">No AirPlay devices found. Press Scan to discover.</div>
            ) : (
              <div class="device-list">
                {airplayDevices.map((device) => (
                  <div class="device-item" key={device.name}>
                    <div class="device-info">
                      <div class="device-name">{device.name}</div>
                      <div class="device-details">
                        <span>{device.model}</span>
                        {airplayActive === device.name && <span class="badge badge-audio">Playing</span>}
                      </div>
                    </div>
                    <div class="device-actions">
                      {airplayActive === device.name ? (
                        <button class="btn btn-sm btn-secondary" onClick={handleAirplayDisconnect}>Stop</button>
                      ) : (
                        <button
                          class="btn btn-sm btn-primary"
                          onClick={() => handleAirplayConnect(device.name)}
                          disabled={airplayActive !== null}
                        >Play</button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
