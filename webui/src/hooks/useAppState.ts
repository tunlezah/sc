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
  device_name: 'SoundSync',
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
        case 'state_snapshot':
          setState(msg.data);
          break;

        case 'device_state_changed':
          setState((prev) => {
            const devices = [...prev.devices];
            const idx = devices.findIndex((d) => d.address === msg.data.address);
            if (idx >= 0) {
              devices[idx] = { ...devices[idx], state: msg.data.state, name: msg.data.name };
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

        case 'bluetooth_status_changed':
          setState((prev) => ({ ...prev, status: msg.data.status }));
          break;
      }
    });

    ws.connect();

    return () => ws.close();
  }, []);

  return { state, spectrum, ws: wsRef };
}
