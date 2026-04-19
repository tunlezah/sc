import { useState, useEffect, useRef } from 'preact/hooks';
import type { AppState, WsMessage } from '../types';
import { WebSocketClient } from '../api/websocket';

const defaultState: AppState = {
  status: 'connecting',
  devices: [],
  eq: { bands: [], enabled: true },
  active_device: null,
  track_info: null,
  playback_status: 'unknown',
  line_in_active: false,
  line_in_available: false,
  device_name: 'Soundy',
  cast_devices: [],
  cast_active: null,
  airplay_devices: [],
  airplay_active: null,
};

export function useAppState() {
  const [state, setState] = useState<AppState>(defaultState);
  const [spectrum, setSpectrum] = useState<number[]>([]);
  const wsRef = useRef<WebSocketClient | null>(null);

  useEffect(() => {
    const ws = new WebSocketClient();
    wsRef.current = ws;

    ws.onMessage((msg: WsMessage) => {
      switch (msg.type) {
        case 'state_snapshot': {
          // Normalize status: BluetoothStatus::Error serializes as {"error":"msg"}
          const snapshot = { ...msg.data };
          if (typeof snapshot.status === 'object' && snapshot.status !== null) {
            snapshot.status = `error:${(snapshot.status as unknown as { error: string }).error}`;
          }
          setState(snapshot);
          break;
        }

        case 'device_state_changed':
          setState((prev) => {
            const devices = [...prev.devices];
            const idx = devices.findIndex((d) => d.address === msg.data.address);
            if (idx >= 0) {
              // Only update name if the new name is non-empty (StreamStarted sends empty name)
              const updatedName = msg.data.name || devices[idx].name;
              devices[idx] = { ...devices[idx], state: msg.data.state, name: updatedName };
            } else {
              devices.push({
                address: msg.data.address,
                name: msg.data.name,
                state: msg.data.state,
                rssi: null,
                trusted: false,
                has_a2dp: false,
                codec: null,
                last_seen: new Date().toISOString(),
                pipewire_node: null,
                // Default to BLE — matches the backend default. The real
                // classification arrives in the next state snapshot.
                type: 'ble',
                is_a2dp_source: false,
              });
            }
            return { ...prev, devices };
          });
          break;

        case 'eq_changed':
          setState((prev) => ({ ...prev, eq: msg.data }));
          break;

        case 'track_changed':
          setState((prev) => ({ ...prev, track_info: msg.data }));
          break;

        case 'playback_status_changed':
          setState((prev) => ({ ...prev, playback_status: msg.data.status }));
          break;

        case 'spectrum_data':
          setSpectrum(msg.data.bands);
          break;

        case 'bluetooth_status_changed': {
          // BluetoothStatus::Error serializes as {"error":"msg"} — normalize to string
          const rawStatus = msg.data.status;
          const status = typeof rawStatus === 'object' && rawStatus !== null
            ? `error:${(rawStatus as { error: string }).error}`
            : rawStatus;
          setState((prev) => ({ ...prev, status }));
          break;
        }

        // Chromecast events
        case 'cast_device_discovered':
          setState((prev) => {
            const castDevices = [...prev.cast_devices];
            const idx = castDevices.findIndex((d) => d.id === msg.data.id);
            if (idx >= 0) {
              castDevices[idx] = msg.data;
            } else {
              castDevices.push(msg.data);
            }
            return { ...prev, cast_devices: castDevices };
          });
          break;

        case 'cast_device_removed':
          setState((prev) => ({
            ...prev,
            cast_devices: prev.cast_devices.filter((d) => d.id !== msg.data.device_id),
          }));
          break;

        case 'cast_session_started':
          setState((prev) => ({ ...prev, cast_active: msg.data.id }));
          break;

        case 'cast_session_stopped':
          setState((prev) => ({ ...prev, cast_active: null }));
          break;

        case 'cast_error':
          console.error('Chromecast error:', msg.data.message);
          break;

        // AirPlay events
        case 'air_play_device_discovered':
          setState((prev) => {
            const airplayDevices = [...prev.airplay_devices];
            const idx = airplayDevices.findIndex((d) => d.name === msg.data.name);
            if (idx >= 0) {
              airplayDevices[idx] = msg.data;
            } else {
              airplayDevices.push(msg.data);
            }
            return { ...prev, airplay_devices: airplayDevices };
          });
          break;

        case 'air_play_device_removed':
          setState((prev) => ({
            ...prev,
            airplay_devices: prev.airplay_devices.filter((d) => d.name !== msg.data.device_name),
          }));
          break;

        case 'air_play_session_started':
          setState((prev) => ({ ...prev, airplay_active: msg.data.name }));
          break;

        case 'air_play_session_stopped':
          setState((prev) => ({ ...prev, airplay_active: null }));
          break;

        case 'air_play_error':
          console.error('AirPlay error:', msg.data.message);
          break;

        case 'line_in_changed':
          setState((prev) => ({ ...prev, line_in_active: msg.data.active }));
          break;
      }
    });

    ws.connect();

    return () => ws.close();
  }, []);

  return { state, spectrum, ws: wsRef };
}
