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
  | { type: 'webrtc_ice_candidate'; data: RTCIceCandidateInit };
