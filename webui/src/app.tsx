import { useAppState } from './hooks/useAppState';
import { useDarkMode } from './hooks/useDarkMode';
import { Header } from './components/Header/Header';
import { DeviceList } from './components/DeviceList/DeviceList';
import { EQControls } from './components/EQControls/EQControls';
import { SpectrumVisualizer } from './components/SpectrumVisualizer/SpectrumVisualizer';
import { MediaControls } from './components/MediaControls/MediaControls';
import { AudioOutput } from './components/AudioOutput/AudioOutput';
import { Settings } from './components/Settings/Settings';

export function App() {
  const { state, spectrum, ws } = useAppState();
  const [dark, toggleDark] = useDarkMode();

  return (
    <div class="app-container">
      <Header
        dark={dark}
        onToggleDark={toggleDark}
        status={state.status}
        lineInActive={state.line_in_active}
        lineInAvailable={state.line_in_available}
      />

      <div class="left-column">
        <SpectrumVisualizer bands={spectrum} dark={dark} />

        <MediaControls
          trackInfo={state.track_info}
          playbackStatus={state.playback_status}
          ws={ws}
        />

        <EQControls
          bands={state.eq.bands}
          enabled={state.eq.enabled}
        />
      </div>

      <div class="right-column">
        <DeviceList
          devices={state.devices}
          activeDevice={state.active_device}
          status={state.status}
        />

        <AudioOutput
          castDevices={state.cast_devices}
          castActive={state.cast_active}
          airplayDevices={state.airplay_devices}
          airplayActive={state.airplay_active}
          devices={state.devices}
        />

        <Settings deviceName={state.device_name} />
      </div>
    </div>
  );
}
