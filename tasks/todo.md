# SoundSync Stability Improvements — Implementation Plan

## Problem Statement

SoundSync has several instability issues:
1. Bluetooth connects but phone doesn't see SoundSync as a speaker (A2DP sink role not applied)
2. PipeWire/WirePlumber fail to restart properly
3. System sometimes requires reboot after install/restart
4. Duplicate/orphaned audio nodes accumulate across restarts
5. No self-healing diagnostic tool exists

## Root Cause Analysis

### Issue 1: Phone doesn't see SoundSync as speaker
- WirePlumber config may be written but not loaded (restart fails silently)
- BlueZ Class of Device must be 0x240414 (Audio/Video + Rendering + Loudspeaker)
- libspa-0.2-bluetooth may be missing
- Bluetooth adapter may not be in discoverable mode

### Issue 2: PipeWire/WirePlumber restart failures
- User services require proper XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS
- systemctl --user from root context fails without su - or machinectl shell
- Service file in scripts/soundsync.service uses hardcoded User=soundsync

### Issue 3: Reboot required
- install.sh restarts WirePlumber but doesn't verify it actually came back
- PipeWire user services may not be enabled for linger
- Runtime dir may not exist after install

### Issue 4: Duplicate nodes
- ExecStartPre cleanup only catches modules with soundsync in args
- Multiple WirePlumber instances can exist
- Orphaned pw-loopback or filter-chain processes from previous runs

## Implementation

### Phase 1: Build soundsync-doctor.sh (comprehensive diagnostic + repair)
### Phase 2: Harden install.sh (verify restarts, auto-recover)
### Phase 3: Fix service file template
### Phase 4: Validate, commit, push
