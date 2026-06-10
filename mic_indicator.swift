import Cocoa
import AVFoundation

let RESULT_PATH = "/tmp/mic_indicator_result"
let SOUND_START = "/System/Library/Sounds/Ping.aiff"
let SOUND_STOP = "/System/Library/Sounds/Pop.aiff"
let SOUND_DONE = "/System/Library/Sounds/Submarine.aiff"
let SOUND_ERROR = "/System/Library/Sounds/Basso.aiff"

private let BAR_W: CGFloat = 260
private let BAR_H: CGFloat = 48
private let BUBBLE_W: CGFloat = 32
private let BUBBLE_H: CGFloat = 32
private let NUM_BARS = 7

// ─── Helpers ───────────────────────────────────────────────────────────────

private func playSound(_ path: String) {
    guard let url = URL(string: "file://\(path)") else { return }
    guard let player = try? AVAudioPlayer(contentsOf: url) else { return }
    player.volume = 0.6
    player.play()
}

private func makeBubble(_ icon: String, color: NSColor, action: Selector?, target: AnyObject?) -> NSButton {
    let b = NSButton(frame: NSRect(x: 0, y: 0, width: BUBBLE_W, height: BUBBLE_H))
    b.title = icon
    b.bezelStyle = .rounded
    b.isBordered = false
    b.wantsLayer = true
    b.layer?.cornerRadius = BUBBLE_W / 2
    b.layer?.backgroundColor = color.cgColor
    b.font = NSFont.systemFont(ofSize: 16, weight: .semibold)
    b.contentTintColor = .white
    b.target = target
    b.action = action
    return b
}

// ─── Indicator ─────────────────────────────────────────────────────────────

class IndicatorWindow {
    private var panel: NSPanel?
    private var mode = "idle"

    // subviews
    private var cancelBtn: NSButton?
    private var confirmBtn: NSButton?
    private var micIcon: NSImageView?
    private var barLayers: [CALayer] = []
    private var spinner: NSProgressIndicator?
    private var doneIcon: NSImageView?

    private var breathing = false
    private var scaledUp = true
    private var currentScale: CGFloat = 1

    // ── public api ──

    func showRecording() {
        mode = "recording"; currentLevel = 0
        DispatchQueue.main.async { self._buildRecordingUI() }
    }

    func showTranscribing() {
        mode = "transcribing"
        DispatchQueue.main.async { self._buildTranscribingUI() }
    }

    func showDone() {
        mode = "done"
        DispatchQueue.main.async { self._buildDoneUI() }
    }

    func updateLevel(_ level: Float) {
        guard mode == "recording", let p = panel, p.isVisible else { return }
        currentLevel = level
        DispatchQueue.main.async { self._animateBars() }
    }

    func hide() {
        mode = "idle"
        DispatchQueue.main.async { self._doHide() }
    }

    // ── recording ui ──

    private var currentLevel: Float = 0

    private func _buildRecordingUI() {
        _ensurePanel()
        guard let p = panel else { return }
        breathing = false; _clear()

        p.setContentSize(NSSize(width: BAR_W, height: BAR_H))

        let root = _makeRoot()
        let content = NSView(frame: NSRect(x: 0, y: 0, width: BAR_W, height: BAR_H))

        // cancel bubble
        let cancel = makeBubble("✕", color: NSColor(red: 0.8, green: 0.2, blue: 0.2, alpha: 0.9), action: #selector(_onCancel), target: self)
        cancelBtn = cancel

        // waveform + mic
        let center = NSView(frame: NSRect(x: 0, y: 0, width: 140, height: BAR_H))
        for i in 0 ..< NUM_BARS {
            let l = CALayer()
            l.backgroundColor = NSColor.white.withAlphaComponent(0.7).cgColor
            l.cornerRadius = 2
            let bw: CGFloat = 4
            let spacing: CGFloat = 6
            let totalW = CGFloat(NUM_BARS) * bw + CGFloat(NUM_BARS - 1) * spacing
            let ox = (140 - totalW) / 2 + CGFloat(i) * (bw + spacing)
            l.frame = NSRect(x: ox, y: (BAR_H - 20) / 2, width: bw, height: 20)
            center.layer?.addSublayer(l)
            barLayers.append(l)
        }
        // mic icon on top of bars
        let mic = NSImageView(frame: NSRect(x: 0, y: 0, width: BAR_H, height: BAR_H))
        if let img = NSImage(systemSymbolName: "mic.fill", accessibilityDescription: nil) {
            let cfg = NSImage.SymbolConfiguration(pointSize: 20, weight: .semibold)
            mic.image = img.withSymbolConfiguration(cfg)
        }
        mic.contentTintColor = .white
        mic.imageScaling = .scaleProportionallyUpOrDown
        micIcon = mic
        center.addSubview(mic)

        // confirm bubble (disabled during recording)
        let confirm = makeBubble("✓", color: NSColor(red: 0.2, green: 0.45, blue: 0.95, alpha: 0.4), action: #selector(_onConfirm), target: self)
        confirm.isEnabled = false
        confirmBtn = confirm

        // layout
        let sv = NSStackView(frame: NSRect(x: 0, y: 0, width: BAR_W, height: BAR_H))
        sv.orientation = .horizontal
        sv.spacing = 8
        sv.alignment = .centerY
        sv.distribution = .fill
        sv.edgeInsets = NSEdgeInsets(top: 0, left: 8, bottom: 0, right: 8)

        sv.addArrangedSubview(cancel)
        let spacerL = NSView(frame: NSRect(x: 0, y: 0, width: 0, height: 1))
        spacerL.setContentHuggingPriority(.defaultLow, for: .horizontal)
        sv.addArrangedSubview(spacerL)
        sv.addArrangedSubview(center)
        let spacerR = NSView(frame: NSRect(x: 0, y: 0, width: 0, height: 1))
        spacerR.setContentHuggingPriority(.defaultLow, for: .horizontal)
        sv.addArrangedSubview(spacerR)
        sv.addArrangedSubview(confirm)

        content.addSubview(sv)
        root.addSubview(content)
        p.contentView = root

        _positionBottom()
        _fadeIn()
        playSound(SOUND_START)
        _startBreathing()
        _animateBars()
    }

    // ── transcribing ui ──

    private func _buildTranscribingUI() {
        _ensurePanel()
        guard let p = panel else { return }
        breathing = false; _clear()

        p.setContentSize(NSSize(width: BAR_W, height: BAR_H))

        let root = _makeRoot()
        let content = NSView(frame: NSRect(x: 0, y: 0, width: BAR_W, height: BAR_H))

        // cancel disabled
        let cancel = makeBubble("✕", color: NSColor(red: 0.5, green: 0.5, blue: 0.5, alpha: 0.5), action: nil, target: nil)
        cancelBtn = cancel

        // spinner
        let sp = NSProgressIndicator(frame: NSRect(x: 0, y: 0, width: 24, height: 24))
        sp.style = .spinning
        sp.controlSize = .regular
        sp.startAnimation(nil)
        spinner = sp

        // confirm disabled
        let confirm = makeBubble("✓", color: NSColor(red: 0.2, green: 0.45, blue: 0.95, alpha: 0.4), action: nil, target: nil)
        confirmBtn = confirm

        // layout
        let sv = NSStackView(frame: NSRect(x: 0, y: 0, width: BAR_W, height: BAR_H))
        sv.orientation = .horizontal
        sv.spacing = 8
        sv.alignment = .centerY
        sv.distribution = .fill
        sv.edgeInsets = NSEdgeInsets(top: 0, left: 8, bottom: 0, right: 8)

        sv.addArrangedSubview(cancel)
        let spacerL = NSView(frame: NSRect(x: 0, y: 0, width: 0, height: 1))
        spacerL.setContentHuggingPriority(.defaultLow, for: .horizontal)
        sv.addArrangedSubview(spacerL)
        sv.addArrangedSubview(sp)
        let spacerR = NSView(frame: NSRect(x: 0, y: 0, width: 0, height: 1))
        spacerR.setContentHuggingPriority(.defaultLow, for: .horizontal)
        sv.addArrangedSubview(spacerR)
        sv.addArrangedSubview(confirm)

        content.addSubview(sv)
        root.addSubview(content)
        p.contentView = root

        _positionBottom()
        p.alphaValue = 0
        p.setIsVisible(true)
        p.orderFront(nil)
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.15
            p.animator().alphaValue = 1
        }
    }

    // ── done ui ──

    private func _buildDoneUI() {
        _ensurePanel()
        guard let p = panel else { return }
        breathing = false; _clear()

        p.setContentSize(NSSize(width: BAR_W, height: BAR_H))

        let root = _makeRoot()
        let content = NSView(frame: NSRect(x: 0, y: 0, width: BAR_W, height: BAR_H))

        // cancel active
        let cancel = makeBubble("✕", color: NSColor(red: 0.8, green: 0.2, blue: 0.2, alpha: 0.9), action: #selector(_onCancel), target: self)
        cancelBtn = cancel

        // checkmark
        let check = NSImageView(frame: NSRect(x: 0, y: 0, width: 32, height: 32))
        if let img = NSImage(systemSymbolName: "checkmark.circle.fill", accessibilityDescription: nil) {
            let cfg = NSImage.SymbolConfiguration(pointSize: 28, weight: .semibold)
            check.image = img.withSymbolConfiguration(cfg)
        }
        check.contentTintColor = NSColor(red: 0.3, green: 0.8, blue: 0.3, alpha: 1)
        check.imageScaling = .scaleProportionallyUpOrDown
        doneIcon = check

        // confirm active
        let confirm = makeBubble("✓", color: NSColor(red: 0.2, green: 0.45, blue: 0.95, alpha: 0.9), action: #selector(_onConfirm), target: self)
        confirmBtn = confirm

        // layout
        let sv = NSStackView(frame: NSRect(x: 0, y: 0, width: BAR_W, height: BAR_H))
        sv.orientation = .horizontal
        sv.spacing = 8
        sv.alignment = .centerY
        sv.distribution = .fill
        sv.edgeInsets = NSEdgeInsets(top: 0, left: 8, bottom: 0, right: 8)

        sv.addArrangedSubview(cancel)
        let spacerL = NSView(frame: NSRect(x: 0, y: 0, width: 0, height: 1))
        spacerL.setContentHuggingPriority(.defaultLow, for: .horizontal)
        sv.addArrangedSubview(spacerL)
        sv.addArrangedSubview(check)
        let spacerR = NSView(frame: NSRect(x: 0, y: 0, width: 0, height: 1))
        spacerR.setContentHuggingPriority(.defaultLow, for: .horizontal)
        sv.addArrangedSubview(spacerR)
        sv.addArrangedSubview(confirm)

        content.addSubview(sv)
        root.addSubview(content)
        p.contentView = root

        _positionBottom()
        p.alphaValue = 0
        p.setIsVisible(true)
        p.orderFront(nil)
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.15
            p.animator().alphaValue = 1
        }
        playSound(SOUND_DONE)
    }

    // ── actions ──

    @objc private func _onCancel() {
        try? "cancel".write(toFile: RESULT_PATH, atomically: true, encoding: .utf8)
        playSound(SOUND_ERROR)
        _doHide()
    }

    @objc private func _onConfirm() {
        try? "confirm".write(toFile: RESULT_PATH, atomically: true, encoding: .utf8)
        _doHide()
    }

    // ── panel management ──

    private func _ensurePanel() {
        guard panel == nil else { return }
        let p = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: BAR_W, height: BAR_H),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        p.isOpaque = false
        p.backgroundColor = .clear
        p.hasShadow = false
        p.level = .floating
        p.ignoresMouseEvents = false
        p.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .transient]
        p.isFloatingPanel = true
        p.hidesOnDeactivate = false
        p.worksWhenModal = true
        self.panel = p
    }

    private func _makeRoot() -> NSView {
        let r = NSView(frame: NSRect(x: 0, y: 0, width: BAR_W, height: BAR_H))
        r.wantsLayer = true

        let bg = CALayer()
        bg.frame = r.bounds
        bg.cornerRadius = BAR_H / 2
        bg.backgroundColor = CGColor(red: 0, green: 0, blue: 0, alpha: 0.65)
        bg.shadowColor = CGColor(red: 0, green: 0, blue: 0, alpha: 0.3)
        bg.shadowOpacity = 1
        bg.shadowOffset = .zero
        bg.shadowRadius = 10
        r.layer?.addSublayer(bg)

        return r
    }

    private func _clear() {
        micIcon = nil; spinner = nil; doneIcon = nil
        barLayers.removeAll()
        panel?.contentView = NSView(frame: .zero)
    }

    private func _positionBottom() {
        guard let screen = NSScreen.main?.visibleFrame else { return }
        let x = screen.minX + (screen.width - BAR_W) / 2
        let y = screen.minY + 12
        panel?.setFrameOrigin(NSPoint(x: x, y: y))
    }

    private func _fadeIn() {
        guard let p = panel else { return }
        p.alphaValue = 0
        p.setIsVisible(true)
        p.orderFront(nil)
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.2
            ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
            p.animator().alphaValue = 1
        }
    }

    private func _doHide() {
        breathing = false
        guard let p = panel else { return }
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.15
            ctx.timingFunction = CAMediaTimingFunction(name: .easeIn)
            p.animator().alphaValue = 0
        } completionHandler: {
            p.setIsVisible(false)
            p.contentView = NSView(frame: .zero)
        }
    }

    // ── waveform bars ──

    private func _animateBars() {
        let baseH: CGFloat = 4
        let maxH: CGFloat = 36
        let level = min(1, currentLevel * 8)

        for (i, layer) in barLayers.enumerated() {
            let phase = sin(CACurrentMediaTime() * 4 + Double(i) * 0.8)
            let variation = CGFloat(phase) * 0.3
            let h = baseH + (maxH - baseH) * min(1, CGFloat(level) + variation + 0.1)
            var f = layer.frame
            f.size.height = max(baseH, min(maxH, h))
            f.origin.y = (BAR_H - f.size.height) / 2
            layer.frame = f
        }
    }

    // ── breathing ──

    private func _startBreathing() {
        guard let p = panel, p.isVisible, !breathing else { return }
        currentScale = 1; scaledUp = true; breathing = true
        _breath()
    }

    private func _breath() {
        guard breathing, let p = panel, p.isVisible else { breathing = false; return }
        let targetScale: CGFloat = scaledUp ? 0.92 : 1.0
        let targetOpacity: CGFloat = scaledUp ? 0.7 : 1.0
        scaledUp.toggle()
        let dur = 0.8

        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = dur
            ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            p.animator().alphaValue = targetOpacity
        }

        if let layer = p.contentView?.layer {
            let anim = CABasicAnimation(keyPath: "transform.scale")
            anim.fromValue = currentScale
            anim.toValue = targetScale
            anim.duration = dur
            anim.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            layer.transform = CATransform3DMakeScale(targetScale, targetScale, 1)
            layer.add(anim, forKey: "breathingScale")
            currentScale = targetScale
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + dur + 0.05) { [weak self] in
            self?._breath()
        }
    }
}

// ─── Socket Server ─────────────────────────────────────────────────────────

class SocketServer {
    private let path: String
    private var sock: Int32 = -1

    init(path: String) { self.path = path }

    func start(indicator: IndicatorWindow) {
        unlink(path)
        sock = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard sock >= 0 else { return }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = path.utf8CString
        let maxPath = MemoryLayout.size(ofValue: addr.sun_path)
        guard bytes.count - 1 < maxPath else { close(sock); sock = -1; return }

        withUnsafeMutablePointer(to: &addr.sun_path.0) { dst in
            bytes.withUnsafeBufferPointer { src in
                UnsafeMutableRawPointer(dst).copyMemory(from: UnsafeRawPointer(src.baseAddress!), byteCount: bytes.count)
            }
        }

        let ok = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(sock, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard ok == 0 else { close(sock); sock = -1; return }

        chmod(path, 0o666)
        listen(sock, 5)

        DispatchQueue.global(qos: .background).async { [weak self] in
            self?.acceptLoop(indicator: indicator)
        }
    }

    private func acceptLoop(indicator: IndicatorWindow) {
        while sock >= 0 {
            let client = accept(sock, nil, nil)
            guard client >= 0 else { continue }
            var buf = [UInt8](repeating: 0, count: 512)
            let n = read(client, &buf, 511)
            var cmd = ""
            if n > 0 { buf[n] = 0; cmd = String(cString: buf).trimmingCharacters(in: .whitespacesAndNewlines) }
            close(client)
            handle(cmd, indicator: indicator)
        }
    }

    private func handle(_ cmd: String, indicator: IndicatorWindow) {
        let parts = cmd.split(separator: " ", maxSplits: 2).map(String.init)
        guard let action = parts.first else { return }
        switch action {
        case "recording":
            indicator.showRecording()
        case "level":
            if parts.count >= 2, let v = Float(parts[1]) {
                indicator.updateLevel(v)
            }
        case "transcribing":
            indicator.showTranscribing()
        case "done":
            indicator.showDone()
        case "hide":
            indicator.hide()
        case "quit":
            DispatchQueue.main.async { NSApplication.shared.terminate(nil) }
        default:
            break
        }
    }

    func stop() {
        if sock >= 0 { close(sock); sock = -1 }
        unlink(path)
    }
}

// ─── Entry ─────────────────────────────────────────────────────────────────

let indicator = IndicatorWindow()
let server = SocketServer(path: "/tmp/mic_indicator.sock")
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
server.start(indicator: indicator)
atexit { server.stop() }
app.run()
