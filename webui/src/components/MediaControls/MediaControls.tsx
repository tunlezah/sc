import { useState, useRef, useEffect, useCallback } from 'preact/hooks';
import type { TrackInfo, PlaybackStatus, WsMessage } from '../../types';
import { WebRTCClient } from '../../api/webrtc';
import type { WebRTCTransport } from '../../api/webrtc';
import * as api from '../../api/rest';

interface MediaControlsProps {
  trackInfo: TrackInfo | null;
  playbackStatus: PlaybackStatus;
  ws: { current: WebRTCTransport | null };
  activeDevice?: string | null;
}

function formatTime(ms: number): string {
  if (!ms || ms <= 0) return '0:00';
  const totalSec = Math.floor(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  return `${min}:${sec.toString().padStart(2, '0')}`;
}

/** Try to start an HTTP audio stream as fallback when WebRTC is unavailable. */
function startHttpStream(): HTMLAudioElement {
  const audio = document.createElement('audio');
  audio.autoplay = true;
  audio.setAttribute('playsinline', '');
  // Safari prefers AAC in ADTS; Chrome handles both.
  // Try AAC first, fall back to MP3 on error.
  audio.src = '/api/stream/audio.aac';
  audio.onerror = () => {
    if (audio.src.includes('/api/stream/audio.aac')) {
      audio.src = '/api/stream/audio.mp3';
    }
  };
  document.body.appendChild(audio);
  return audio;
}

export function MediaControls({ trackInfo, playbackStatus, ws, activeDevice }: MediaControlsProps) {
  const isPlaying = playbackStatus === 'playing';
  const [listening, setListening] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const rtcRef = useRef<WebRTCClient | null>(null);
  const httpAudioRef = useRef<HTMLAudioElement | null>(null);
  const unsubRef = useRef<(() => void) | null>(null);
  const elapsedRef = useRef(0);
  const trackStartRef = useRef<number>(Date.now());

  // Track elapsed time when playing
  useEffect(() => {
    if (isPlaying && trackInfo?.duration_ms) {
      trackStartRef.current = Date.now() - elapsedRef.current;
      const interval = setInterval(() => {
        const now = Date.now() - trackStartRef.current;
        const clamped = Math.min(now, trackInfo.duration_ms);
        elapsedRef.current = clamped;
        setElapsed(clamped);
      }, 500);
      return () => clearInterval(interval);
    }
  }, [isPlaying, trackInfo?.duration_ms, trackInfo?.title]);

  // Reset elapsed on track change — use position_ms from AVRCP if available
  // so the progress bar starts at the right point for mid-track joins
  useEffect(() => {
    const startPos = trackInfo?.position_ms ?? 0;
    elapsedRef.current = startPos;
    setElapsed(startPos);
    trackStartRef.current = Date.now() - startPos;
  }, [trackInfo?.title, trackInfo?.artist]);

  const cleanup = useCallback(() => {
    rtcRef.current?.stop();
    rtcRef.current = null;
    if (httpAudioRef.current) {
      httpAudioRef.current.pause();
      httpAudioRef.current.removeAttribute('src');
      httpAudioRef.current.remove();
      httpAudioRef.current = null;
    }
    if (unsubRef.current) {
      unsubRef.current();
      unsubRef.current = null;
    }
    setListening(false);
  }, []);

  const handleListenToggle = async () => {
    if (listening) {
      cleanup();
    } else {
      if (ws.current) {
        const client = new WebRTCClient(ws.current, (playing) => {
          if (!playing && !httpAudioRef.current) {
            // WebRTC disconnected unexpectedly — don't fall back automatically
            setListening(false);
          } else {
            setListening(playing);
          }
        });
        rtcRef.current = client;

        unsubRef.current = ws.current.onMessage((msg: WsMessage) => {
          if (msg.type === 'webrtc_answer') {
            client.handleAnswer(msg.data.sdp);
          } else if (msg.type === 'webrtc_ice_candidate') {
            client.handleIceCandidate(msg.data);
          }
        });

        try {
          await client.start();
          setListening(true);

          // Set a timeout: if WebRTC doesn't connect within 5s, fall back to HTTP stream
          setTimeout(() => {
            if (rtcRef.current && !rtcRef.current.isActive && !httpAudioRef.current) {
              console.warn('WebRTC connection timed out, falling back to HTTP stream');
              rtcRef.current.stop();
              rtcRef.current = null;
              const audio = startHttpStream();
              httpAudioRef.current = audio;
              audio.play().catch(() => {});
            }
          }, 5000);
        } catch {
          // WebRTC failed immediately — use HTTP stream fallback
          rtcRef.current = null;
          const audio = startHttpStream();
          httpAudioRef.current = audio;
          audio.play().then(() => setListening(true)).catch(() => setListening(false));
        }
      } else {
        // No WebSocket available — use HTTP stream directly
        const audio = startHttpStream();
        httpAudioRef.current = audio;
        audio.play().then(() => setListening(true)).catch(() => setListening(false));
      }
    }
  };

  useEffect(() => {
    return () => cleanup();
  }, [cleanup]);

  const progressPct = trackInfo?.duration_ms
    ? Math.min((elapsed / trackInfo.duration_ms) * 100, 100)
    : 0;

  return (
    <div class="media-controls-row" style={{ flexWrap: 'wrap' }}>
      <div class="media-track-info">
        {trackInfo ? (
          <>
            <div class="media-track-title">{trackInfo.title || 'Unknown Track'}</div>
            <div class="media-track-artist">
              {trackInfo.artist || 'Unknown Artist'}
              {trackInfo.album ? ` \u2014 ${trackInfo.album}` : ''}
              {activeDevice ? <span style={{ marginLeft: '6px', opacity: 0.6, fontSize: '0.7rem' }}>via Bluetooth</span> : ''}
            </div>
          </>
        ) : (
          <div class="media-track-title" style={{ color: 'var(--text-secondary)' }}>
            No track playing
          </div>
        )}
      </div>

      <div class="media-buttons">
        <button class="media-btn" onClick={() => api.avrcpPrevious()} title="Previous">
          {'\u23EE'}
        </button>
        <button
          class="media-btn media-btn-play"
          onClick={() => (isPlaying ? api.avrcpPause() : api.avrcpPlay())}
          title={isPlaying ? 'Pause' : 'Play'}
        >
          {isPlaying ? '\u23F8' : '\u25B6'}
        </button>
        <button class="media-btn" onClick={() => api.avrcpNext()} title="Next">
          {'\u23ED'}
        </button>
      </div>

      <button
        class={`btn btn-sm media-listen-btn ${listening ? 'btn-danger' : 'btn-primary'}`}
        onClick={handleListenToggle}
        title={listening ? 'Stop listening in browser' : 'Listen in browser'}
      >
        {listening ? '\u23F9 Stop' : '\u{1F50A} Listen'}
      </button>

      {trackInfo?.duration_ms ? (
        <div class="media-progress">
          <span class="media-progress-time">{formatTime(elapsed)}</span>
          <div class="media-progress-bar">
            <div class="media-progress-fill" style={{ width: `${progressPct}%` }} />
          </div>
          <span class="media-progress-time">{formatTime(trackInfo.duration_ms)}</span>
        </div>
      ) : null}
    </div>
  );
}
