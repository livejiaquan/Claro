#!/bin/bash
swiftc -o mic_indicator mic_indicator.swift -framework Cocoa -framework AVFoundation
echo "Built mic_indicator"
