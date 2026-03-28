import { useState, useRef, useEffect, useCallback } from 'preact/hooks';
import type { TrackInfo, PlaybackStatus, WsMessage } from '../../types';
import { WebRTCClient } from '../../api/webrtc';
import type { WebRTCTransport } from '../../api/webrtc';
import * as api from '../../api/rest';

interface MediaControlsProps {
  trackInfo: TrackInfo | null;
  playbackStatus: PlaybackStatus;
  ws: { current: WebRTCTransport | null };
}

/** Try to start an HTTP audio stream as fallback when WebRTC is unavailable. */
function startHttpStream(): HTMLAudioElement {
  const audio = document.createElement('audio');
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

export function MediaControls({ trackInfo, playbackStatus, ws }: MediaControlsProps) {
  const isPlaying = playbackStatus === 'playing';
  const [listening, setListening] = useState(false);
  const rtcRef = useRef<WebRTCClient | null>(null);
  const httpAudioRef = useRef<HTMLAudioElement | null>(null);
  const unsubRef = useRef<(() => void) | null>(null);

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

  return (
    <div class="media-controls-row">
      <div class="media-track-info">
        {trackInfo ? (
          <>
            <div class="media-track-title">{trackInfo.title || 'Unknown Track'}</div>
            <div class="media-track-artist">
              {trackInfo.artist || 'Unknown Artist'}
              {trackInfo.album ? ` \u2014 ${trackInfo.album}` : ''}
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
    </div>
  );
}
