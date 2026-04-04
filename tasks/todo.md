# SoundSync Tasks — Current Session

## Task 1: Fix Safari Audio
- [ ] Improve WebRTC audio element creation for Safari gesture chain
- [ ] Ensure HTTP AAC fallback triggers properly on Safari WebRTC failure

## Task 2: Fix Line-In Shown Twice
- [ ] Remove Line-In tab from AudioOutput (it's an input, not output)
- [ ] Remove Line-In toggle from Header
- [ ] Merge Bluetooth + Line-In into unified "Audio Input" section with tabs
- [ ] Show active input indicator in header
- [ ] Backend: deactivate line-in when BT connects, and vice versa

## Task 3: Add System Theme Option
- [ ] Modify useDarkMode hook for 'light' | 'dark' | 'system' modes
- [ ] Add matchMedia listener for system preference changes
- [ ] Update Header to show 3-option theme selector
- [ ] Make "system" the default for new users

## Task 4: Fix Device Name Not Persisting
- [ ] Fix Settings component to sync with prop changes on state_snapshot

## Task 5: Fix Bluetooth Scan Not Stopping
- [ ] Send StopScan when AVRCP play is pressed
- [ ] Frontend: stop scan animation on device connect/audio_active

## Task 6: Fix AirPlay pactl load-module failure
- [ ] Fix: use PipeWire native RAOP approach instead of non-existent module-raop-sink
- [ ] Properly discover and connect to RAOP sinks via pw-link

## Final
- [ ] Run CI checks
- [ ] Update version, README, commit, push
