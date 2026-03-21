export interface WebRTCTransport {
  send(data: unknown): void;
  onMessage(handler: (msg: import('../types').WsMessage) => void): () => void;
}

export class WebRTCClient {
  private pc: RTCPeerConnection | null = null;
  private ws: WebRTCTransport;
  private audioElement: HTMLAudioElement | null = null;
  private onStateChange: (playing: boolean) => void;

  constructor(ws: WebRTCTransport, onStateChange: (playing: boolean) => void) {
    this.ws = ws;
    this.onStateChange = onStateChange;
  }

  async start(): Promise<void> {
    this.pc = new RTCPeerConnection({
      iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
    });

    this.pc.ontrack = (event) => {
      if (!this.audioElement) {
        this.audioElement = document.createElement('audio');
        this.audioElement.autoplay = true;
      }
      this.audioElement.srcObject = event.streams[0];
      this.audioElement.play().catch(() => {
        // autoplay may be blocked
      });
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
    }
  }

  async handleIceCandidate(candidate: RTCIceCandidateInit): Promise<void> {
    if (this.pc) {
      await this.pc.addIceCandidate(new RTCIceCandidate(candidate));
    }
  }

  stop(): void {
    if (this.audioElement) {
      this.audioElement.pause();
      this.audioElement.srcObject = null;
    }
    this.pc?.close();
    this.pc = null;
    this.onStateChange(false);
    this.ws.send({ type: 'webrtc_stop', data: {} });
  }

  get isActive(): boolean {
    return this.pc !== null && this.pc.connectionState === 'connected';
  }
}
