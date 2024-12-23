#!/usr/bin/env fish

# Allow specifying the client binary path as an argument
if test (count $argv) -gt 0
    set client_path $argv[1]
else
    # Default to looking in PATH
    set client_path "wayland-osd-client"
end

# Verify client exists and is executable
if not type -q $client_path; and not test -x $client_path
    echo "Error: wayland-osd-client not found or not executable at '$client_path'"
    exit 1
end

# Get the volume info from wpctl
set volume_info (wpctl get-volume @DEFAULT_AUDIO_SINK@)
if test $status -ne 0
    echo "Error: Failed to get volume information from wpctl"
    exit 1
end

# Extract volume value and mute state
# Example output from wpctl: "Volume: 0.75" or "Volume: 0.50 [MUTED]"
set volume_parts (string split " " $volume_info)
if test (count $volume_parts) -lt 2
    echo "Error: Unexpected wpctl output format: $volume_info"
    exit 1
end

set volume_float (string replace "Volume:" "" $volume_parts[2])
if not string match -qr '^[0-9]+(\.[0-9]+)?$' $volume_float
    echo "Error: Invalid volume value: $volume_float"
    exit 1
end

set is_muted (string match -r '\[MUTED\]' $volume_info)

# Convert float volume (0.0-1.0) to percentage (0-100)
set volume_percent (math "round($volume_float * 100)")

if test -n "$is_muted"
    # Show muted state
    $client_path audio --mute $volume_percent
else
    # Show volume level
    $client_path audio $volume_percent
end