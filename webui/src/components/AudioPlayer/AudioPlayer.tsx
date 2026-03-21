import { useState, useRef, useEffect } from 'preact/hooks';
import { WebRTCClient } from '../../api/webrtc';
import type { WebSocketClient } from '../../api/websocket';
import type { MutableRef } from 'preact/hooks';

interface AudioPlayerProps {
  ws: MutableRef<WebSocketClient | null>;
}

export function AudioPlayer({ ws }: AudioPlayerProps) {
  const [listening, setListening] = useState(false);
  const rtcRef = useRef<WebRTCClient | null>(null);

  const handleToggle = async () => {
    if (listening) {
      rtcRef.current?.stop();
      rtcRef.current = null;
      setListening(false);
    } else {
      if (ws.current) {
        const client = new WebRTCClient(ws.current, setListening);
        rtcRef.current = client;

        // Listen for answers and ICE candidates
        const _unsub = ws.current.onMessage((msg) => {
          if (msg.type === 'webrtc_answer') {
            client.handleAnswer(msg.data.sdp);
          } else if (msg.type === 'webrtc_ice_candidate') {
            client.handleIceCandidate(msg.data);
          }
        });

        try {
          await client.start();
          setListening(true);
        } catch {
          setListening(false);
        }
      }
    }
  };

  useEffect(() => {
    return () => {
      rtcRef.current?.stop();
    };
  }, []);

  return (
    <div class="card">
      <div class="card-content">
        <div class="audio-player">
          <button
            class={`btn ${listening ? 'btn-danger' : 'btn-primary'}`}
            onClick={handleToggle}
          >
            {listening ? '\u23F9 Stop Listening' : '\u{1F50A} Listen in Browser'}
          </button>
          {listening && (
            <span style={{ fontSize: '0.85rem', color: 'var(--success)' }}>
              Streaming audio...
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
