import type { TrackInfo, PlaybackStatus } from '../../types';
import * as api from '../../api/rest';

interface MediaControlsProps {
  trackInfo: TrackInfo | null;
  playbackStatus: PlaybackStatus;
}

export function MediaControls({ trackInfo, playbackStatus }: MediaControlsProps) {
  const isPlaying = playbackStatus === 'playing';

  return (
    <div class="card">
      <div class="card-content">
        <div class="media-controls">
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
              class={`media-btn media-btn-play`}
              onClick={() => (isPlaying ? api.avrcpPause() : api.avrcpPlay())}
              title={isPlaying ? 'Pause' : 'Play'}
            >
              {isPlaying ? '\u23F8' : '\u25B6'}
            </button>
            <button class="media-btn" onClick={() => api.avrcpNext()} title="Next">
              {'\u23ED'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
