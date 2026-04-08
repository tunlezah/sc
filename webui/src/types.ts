export type DeviceState =
  | 'disconnected'
  | 'discovered'
  | 'pairing'
  | 'paired'
  | 'connected'
  | 'profile_negotiated'
  | 'pipewire_source_ready'
  | 'audio_active';

export type PlaybackStatus = 'playing' | 'paused' | 'stopped' | 'unknown';

export type AudioCodec = 'sbc' | 'aac' | 'ldac' | 'apt_x' | 'apt_x_hd';

export type DeviceKind = 'classic' | 'ble';

export interface DeviceInfo {
  address: string;
  name: string;
  state: DeviceState;
  rssi: number | null;
  trusted: boolean;
  has_a2dp: boolean;
  codec: AudioCodec | null;
  last_seen: string;
  pipewire_node: string | null;
  /** BLE vs Classic Bluetooth (best-effort classification). */
  type: DeviceKind;
  /** Best-effort: device advertises A2DP source UUID. */
  is_a2dp_source: boolean;
}

export interface EqBand {
  freq: number;
  gain_db: number;
  q: number;
  filter_type: 'low_shelf' | 'peaking' | 'high_shelf';
}

export interface TrackInfo {
  title: string;
  artist: string;
  album: string;
  duration_ms: number;
  track_number: number | null;
  artwork_url?: string;
}

export interface CastDevice {
  id: string;
  name: string;
  address: string;
  port: number;
  model: string;
}

export interface AirPlayDevice {
  name: string;
  address: string;
  port: number;
  model: string;
}

export interface AppState {
  status: string;
  devices: DeviceInfo[];
  eq: { bands: EqBand[]; enabled: boolean };
  active_device: string | null;
  track_info: TrackInfo | null;
  playback_status: PlaybackStatus;
  line_in_active: boolean;
  line_in_available: boolean;
  device_name: string;
  cast_devices: CastDevice[];
  cast_active: string | null;
  airplay_devices: AirPlayDevice[];
  airplay_active: string | null;
}

// ICE candidate from the server — may use camelCase or snake_case keys
export interface IceCandidateMessage {
  candidate: string;
  sdpMid?: string | null;
  sdpMLineIndex?: number | null;
  sdp_mid?: string | null;
  sdp_mline_index?: number | null;
}

export type WsMessage =
  | { type: 'state_snapshot'; data: AppState }
  | { type: 'device_state_changed'; data: { address: string; name: string; state: DeviceState } }
  | { type: 'eq_changed'; data: { bands: EqBand[]; enabled: boolean } }
  | { type: 'track_changed'; data: TrackInfo | null }
  | { type: 'playback_status_changed'; data: { status: PlaybackStatus } }
  | { type: 'spectrum_data'; data: { bands: number[] } }
  | { type: 'bluetooth_status_changed'; data: { status: string } }
  | { type: 'webrtc_answer'; data: { sdp: string } }
  | { type: 'webrtc_ice_candidate'; data: IceCandidateMessage }
  | { type: 'cast_device_discovered'; data: CastDevice }
  | { type: 'cast_device_removed'; data: { device_id: string } }
  | { type: 'cast_session_started'; data: CastDevice }
  | { type: 'cast_session_stopped'; data: { device_id: string } }
  | { type: 'cast_error'; data: { message: string } }
  | { type: 'air_play_device_discovered'; data: AirPlayDevice }
  | { type: 'air_play_device_removed'; data: { device_name: string } }
  | { type: 'air_play_session_started'; data: AirPlayDevice }
  | { type: 'air_play_session_stopped'; data: { device_name: string } }
  | { type: 'air_play_error'; data: { message: string } }
  | { type: 'line_in_changed'; data: { active: boolean } };
