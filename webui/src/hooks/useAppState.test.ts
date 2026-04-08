import { describe, it, expect } from 'vitest';
import type { AppState, WsMessage } from '../types';

// Test the WebSocket message handling logic in isolation.
// We extract the reducer logic and test it without needing a real WebSocket.

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
  cast_devices: [],
  cast_active: null,
  airplay_devices: [],
  airplay_active: null,
};

/**
 * Simulate the message handler from useAppState.
 * This mirrors the switch/case logic in useAppState.ts so we can test it
 * without mounting a component or opening a WebSocket.
 */
function applyMessage(prev: AppState, msg: WsMessage): AppState {
  switch (msg.type) {
    case 'state_snapshot':
      return msg.data;

    case 'device_state_changed': {
      const devices = [...prev.devices];
      const idx = devices.findIndex((d) => d.address === msg.data.address);
      if (idx >= 0) {
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
          type: 'classic',
          is_a2dp_source: false,
        });
      }
      return { ...prev, devices };
    }

    case 'bluetooth_status_changed':
      return { ...prev, status: msg.data.status };

    case 'line_in_changed':
      return { ...prev, line_in_active: msg.data.active };

    default:
      return prev;
  }
}

// ── Bluetooth Scanning Tests ──────────────────────────────────────

describe('Bluetooth Scanning', () => {
  it('scan start changes status to scanning', () => {
    const state = applyMessage(defaultState, {
      type: 'bluetooth_status_changed',
      data: { status: 'scanning' },
    });
    expect(state.status).toBe('scanning');
  });

  it('scan stop changes status back to ready', () => {
    const scanning = { ...defaultState, status: 'scanning' };
    const state = applyMessage(scanning, {
      type: 'bluetooth_status_changed',
      data: { status: 'ready' },
    });
    expect(state.status).toBe('ready');
  });

  it('discovered device is added to device list', () => {
    const state = applyMessage(defaultState, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:FF', name: 'TestPhone', state: 'discovered' },
    });
    expect(state.devices).toHaveLength(1);
    expect(state.devices[0].address).toBe('AA:BB:CC:DD:EE:FF');
    expect(state.devices[0].name).toBe('TestPhone');
    expect(state.devices[0].state).toBe('discovered');
  });

  it('multiple discovered devices accumulate', () => {
    let state = defaultState;
    state = applyMessage(state, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:01', name: 'Phone1', state: 'discovered' },
    });
    state = applyMessage(state, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:02', name: 'Phone2', state: 'discovered' },
    });
    state = applyMessage(state, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:03', name: 'Phone3', state: 'discovered' },
    });
    expect(state.devices).toHaveLength(3);
  });

  it('device without name falls back gracefully (empty string)', () => {
    const state = applyMessage(defaultState, {
      type: 'device_state_changed',
      data: { address: '11:22:33:44:55:66', name: '', state: 'discovered' },
    });
    expect(state.devices[0].name).toBe('');
    // Frontend renders: device.name || device.address
    expect(state.devices[0].name || state.devices[0].address).toBe('11:22:33:44:55:66');
  });

  it('deferred name update preserves existing name when empty', () => {
    let state = applyMessage(defaultState, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:FF', name: 'MyPhone', state: 'discovered' },
    });
    // Backend sends a state change with empty name (e.g. StreamStarted)
    state = applyMessage(state, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:FF', name: '', state: 'audio_active' },
    });
    // Name should be preserved, not overwritten with empty
    expect(state.devices[0].name).toBe('MyPhone');
    expect(state.devices[0].state).toBe('audio_active');
  });

  it('deferred name update applies real name', () => {
    let state = applyMessage(defaultState, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:FF', name: '', state: 'discovered' },
    });
    // Poll resolves the name
    state = applyMessage(state, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:FF', name: 'Living Room Speaker', state: 'discovered' },
    });
    expect(state.devices[0].name).toBe('Living Room Speaker');
  });

  it('scan error sets error status', () => {
    const state = applyMessage(defaultState, {
      type: 'bluetooth_status_changed',
      data: { status: 'error' },
    });
    expect(state.status).toBe('error');
  });

  it('state_snapshot replaces entire state', () => {
    const snapshot: AppState = {
      ...defaultState,
      status: 'scanning',
      devices: [
        {
          address: 'AA:BB:CC:DD:EE:FF',
          name: 'TestPhone',
          state: 'connected',
          rssi: -50,
          trusted: true,
          has_a2dp: true,
          codec: null,
          last_seen: '2026-01-01T00:00:00Z',
          pipewire_node: null,
          type: 'classic',
          is_a2dp_source: false,
        },
      ],
    };
    const state = applyMessage(defaultState, {
      type: 'state_snapshot',
      data: snapshot,
    });
    expect(state.status).toBe('scanning');
    expect(state.devices).toHaveLength(1);
    expect(state.devices[0].name).toBe('TestPhone');
  });
});

// ── Line-In Input Source Tests ────────────────────────────────────

describe('Line-In Input Source', () => {
  it('line_in_changed activates line-in', () => {
    const state = applyMessage(defaultState, {
      type: 'line_in_changed',
      data: { active: true },
    });
    expect(state.line_in_active).toBe(true);
  });

  it('line_in_changed deactivates line-in', () => {
    const active = { ...defaultState, line_in_active: true };
    const state = applyMessage(active, {
      type: 'line_in_changed',
      data: { active: false },
    });
    expect(state.line_in_active).toBe(false);
  });

  it('only one input source state at a time via state_snapshot', () => {
    // When line-in is active, bluetooth active_device should be null
    const snapshot: AppState = {
      ...defaultState,
      line_in_active: true,
      active_device: null,
    };
    const state = applyMessage(defaultState, {
      type: 'state_snapshot',
      data: snapshot,
    });
    expect(state.line_in_active).toBe(true);
    expect(state.active_device).toBeNull();
  });

  it('switching from line-in to bluetooth clears line-in', () => {
    let state: AppState = { ...defaultState, line_in_active: true };
    // Line-in deactivated
    state = applyMessage(state, {
      type: 'line_in_changed',
      data: { active: false },
    });
    // Bluetooth device connected
    state = applyMessage(state, {
      type: 'device_state_changed',
      data: { address: 'AA:BB:CC:DD:EE:FF', name: 'Speaker', state: 'audio_active' },
    });
    expect(state.line_in_active).toBe(false);
    expect(state.devices[0].state).toBe('audio_active');
  });
});
