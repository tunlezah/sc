import { useState, useRef, useEffect, useCallback } from 'preact/hooks';
import { WebRTCClient } from '../../api/webrtc';
import type { WsMessage } from '../../types';

interface AudioPlayerProps {
  ws: { current: { send: (msg: unknown) => void; onMessage: (handler: (msg: WsMessage) => void) => () => void } | null };
}

export function AudioPlayer({ ws }: AudioPlayerProps) {
  const [listening, setListening] = useState(false);
  const rtcRef = useRef<WebRTCClient | null>(null);
  const unsubRef = useRef<(() => void) | null>(null);

  // Cleanup function to stop WebRTC and unsubscribe from WS messages
  const cleanup = useCallback(() => {
    rtcRef.current?.stop();
    rtcRef.current = null;
    // Remove the WebSocket message handler to prevent leak
    if (unsubRef.current) {
      unsubRef.current();
      unsubRef.current = null;
    }
    setListening(false);
  }, []);

  const handleToggle = async () => {
    if (listening) {
      cleanup();
    } else {
      if (ws.current) {
        const client = new WebRTCClient(ws.current, setListening);
        rtcRef.current = client;

        // Subscribe to WebRTC signaling messages and store the unsubscribe fn
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
        } catch {
          cleanup();
        }
      }
    }
  };

  // Cleanup on unmount
  useEffect(() => {
    return () => cleanup();
  }, [cleanup]);

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
