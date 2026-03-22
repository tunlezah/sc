import type { DeviceInfo, EqBand, CastDevice, AirPlayDevice } from '../types';

const BASE = '';

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status}`);
  return res.json();
}

// System
export const getStatus = () => request<{ status: string; device_count: number; uptime_secs: number }>('GET', '/api/status');
export const getDevices = () => request<DeviceInfo[]>('GET', '/api/devices');

// Bluetooth
export const startScan = () => request<{ ok: boolean }>('POST', '/api/bluetooth/scan', { scanning: true });
export const stopScan = () => request<{ ok: boolean }>('POST', '/api/bluetooth/scan', { scanning: false });
export const connectDevice = (address: string) => request<{ ok: boolean }>('POST', '/api/bluetooth/connect', { address });
export const disconnectDevice = (address: string) => request<{ ok: boolean }>('POST', '/api/bluetooth/disconnect', { address });
export const removeDevice = (address: string) => request<{ ok: boolean }>('DELETE', '/api/bluetooth/device', { address });
export const setDeviceName = (name: string) => request<{ ok: boolean }>('POST', '/api/bluetooth/name', { name });

// EQ
export const getEq = () => request<{ bands: EqBand[]; enabled: boolean }>('GET', '/api/eq');
export const updateEq = (bands: { gain_db: number }[], enabled?: boolean) =>
  request<{ ok: boolean }>('POST', '/api/eq', { bands, enabled });
export const getPresets = () => request<string[]>('GET', '/api/eq/presets');
export const applyPreset = (name: string) => request<{ ok: boolean }>('POST', '/api/eq/preset', { name });
export const savePreset = (name: string) => request<{ ok: boolean }>('POST', '/api/eq/preset/save', { name });
export const deletePreset = (name: string) => request<{ ok: boolean }>('DELETE', `/api/eq/preset/${encodeURIComponent(name)}`);

// Line-in
export const getLineInStatus = () => request<{ available: boolean; active: boolean; source_name: string | null }>('GET', '/api/line-in/status');
export const activateLineIn = () => request<{ ok: boolean }>('POST', '/api/line-in/activate');
export const deactivateLineIn = () => request<{ ok: boolean }>('POST', '/api/line-in/deactivate');

// AVRCP
export const avrcpPlay = () => request<{ ok: boolean }>('POST', '/api/avrcp/play');
export const avrcpPause = () => request<{ ok: boolean }>('POST', '/api/avrcp/pause');
export const avrcpNext = () => request<{ ok: boolean }>('POST', '/api/avrcp/next');
export const avrcpPrevious = () => request<{ ok: boolean }>('POST', '/api/avrcp/previous');

// Chromecast
export const getCastDevices = () => request<CastDevice[]>('GET', '/api/cast/devices');
export const castDiscover = () => request<{ ok: boolean }>('POST', '/api/cast/discover');
export const castConnect = (deviceId: string) => request<{ ok: boolean }>('POST', '/api/cast/connect', { device_id: deviceId });
export const castDisconnect = () => request<{ ok: boolean }>('POST', '/api/cast/disconnect');
export const castVolume = (level: number) => request<{ ok: boolean }>('POST', '/api/cast/volume', { level });

// AirPlay
export const getAirplayDevices = () => request<AirPlayDevice[]>('GET', '/api/airplay/devices');
export const airplayDiscover = () => request<{ ok: boolean }>('POST', '/api/airplay/discover');
export const airplayConnect = (name: string) => request<{ ok: boolean }>('POST', '/api/airplay/connect', { name });
export const airplayDisconnect = () => request<{ ok: boolean }>('POST', '/api/airplay/disconnect');
export const airplayVolume = (level: number) => request<{ ok: boolean }>('POST', '/api/airplay/volume', { level });
