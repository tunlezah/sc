export interface WebRTCTransport {
  send(data: unknown): void;
  onMessage(handler: (msg: import('../types').WsMessage) => void): () => void;
}

export class WebRTCClient {
  private pc: RTCPeerConnection | null = null;
  private ws: WebRTCTransport;
  private audioElement: HTMLAudioElement | null = null;
  private onStateChange: (playing: boolean) => void;
  // Safari throws InvalidStateError if addIceCandidate() is called before
  // setRemoteDescription() completes. Chrome/Firefox silently queue candidates,
  // but Safari does not. Buffer candidates until the remote description is set.
  private remoteDescriptionSet = false;
  private pendingCandidates: RTCIceCandidateInit[] = [];

  constructor(ws: WebRTCTransport, onStateChange: (playing: boolean) => void) {
    this.ws = ws;
    this.onStateChange = onStateChange;
  }

  async start(): Promise<void> {
    // Create audio element and call play() synchronously during the user
    // gesture (click handler). Safari's autoplay policy requires play() to
    // be called BEFORE any `await` — the first await breaks the synchronous
    // gesture chain and subsequent play() calls are blocked as non-user-initiated.
    this.audioElement = document.createElement('audio');
    this.audioElement.autoplay = true;
    this.audioElement.setAttribute('playsinline', '');
    document.body.appendChild(this.audioElement);

    // Prime the audio element NOW, before any async work. This must be the
    // first interaction with the element and must happen synchronously in the
    // gesture context. Without this, Safari blocks all subsequent play() calls.
    this.audioElement.play().catch(() => {});

    // Unlock AudioContext for Safari. This await may break the gesture chain,
    // but the audio element is already primed above so playback will work.
    try {
      const AudioCtx = window.AudioContext || (window as any).webkitAudioContext;
      if (AudioCtx) {
        const ctx = new AudioCtx();
        await ctx.resume();
        ctx.close();
      }
    } catch {
      // AudioContext not available — continue without it
    }

    this.pc = new RTCPeerConnection({
      iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
    });

    this.pc.ontrack = (event) => {
      if (this.audioElement) {
        // Use event.track directly — webrtc-rs add_track() doesn't associate
        // tracks with streams, so event.streams[0] is undefined.
        const stream = event.streams[0] ?? new MediaStream([event.track]);
        this.audioElement.srcObject = stream;

        this.audioElement.play().catch((err) => {
          console.warn('WebRTC audio play() failed:', err.message);
          // On Safari, try toggling muted state to unlock playback
          if (this.audioElement) {
            this.audioElement.muted = true;
            this.audioElement.play().then(() => {
              if (this.audioElement) this.audioElement.muted = false;
            }).catch(() => {});
          }
        });
      }
      this.onStateChange(true);
    };

    this.pc.onicecandidate = (event) => {
      if (event.candidate) {
        this.ws.send({
          type: 'webrtc_ice_candidate',
          data: event.candidate.toJSON(),
        });
      }
    };

    this.pc.onconnectionstatechange = () => {
      if (this.pc?.connectionState === 'disconnected' || this.pc?.connectionState === 'failed') {
        this.onStateChange(false);
      }
    };

    // Create offer (recvonly audio)
    this.pc.addTransceiver('audio', { direction: 'recvonly' });
    const offer = await this.pc.createOffer();
    await this.pc.setLocalDescription(offer);

    this.ws.send({
      type: 'webrtc_offer',
      data: { sdp: offer.sdp },
    });
  }

  async handleAnswer(sdp: string): Promise<void> {
    if (this.pc) {
      await this.pc.setRemoteDescription(new RTCSessionDescription({ type: 'answer', sdp }));
      this.remoteDescriptionSet = true;

      // Flush any ICE candidates that arrived before setRemoteDescription completed.
      // This is the fix for Safari — it rejects addIceCandidate() calls made before
      // the remote description is set, unlike Chrome/Firefox which queue them.
      for (const init of this.pendingCandidates) {
        await this.pc.addIceCandidate(new RTCIceCandidate(init));
      }
      this.pendingCandidates = [];
    }
  }

  async handleIceCandidate(candidate: import('../types').IceCandidateMessage): Promise<void> {
    if (!this.pc) return;

    // Server may send snake_case (sdp_mid, sdp_mline_index) or camelCase
    // (sdpMid, sdpMLineIndex). Normalize to camelCase for RTCIceCandidateInit.
    const sdpMid = candidate.sdpMid ?? candidate.sdp_mid ?? null;
    const sdpMLineIndex = candidate.sdpMLineIndex ?? candidate.sdp_mline_index ?? null;

    // Safari strictly requires at least one of sdpMid or sdpMLineIndex to be non-null.
    // Drop candidates that violate this — they are end-of-candidates signals or malformed.
    if (sdpMid == null && sdpMLineIndex == null) {
      return;
    }

    const init: RTCIceCandidateInit = {
      candidate: candidate.candidate,
      sdpMid,
      sdpMLineIndex,
    };

    if (!this.remoteDescriptionSet) {
      this.pendingCandidates.push(init);
      return;
    }

    await this.pc.addIceCandidate(new RTCIceCandidate(init));
  }

  stop(): void {
    if (this.audioElement) {
      this.audioElement.pause();
      this.audioElement.srcObject = null;
      this.audioElement.remove();
      this.audioElement = null;
    }
    this.pc?.close();
    this.pc = null;
    this.remoteDescriptionSet = false;
    this.pendingCandidates = [];
    this.onStateChange(false);
    this.ws.send({ type: 'webrtc_stop', data: {} });
  }

  get isActive(): boolean {
    return this.pc !== null && this.pc.connectionState === 'connected';
  }
}
