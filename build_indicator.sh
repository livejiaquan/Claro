#!/bin/bash
swiftc -o "$(dirname "$0")/mic_indicator" "$(dirname "$0")/mic_indicator.swift" -framework Cocoa
echo "Built mic_indicator"
