import Cocoa

// ─── Constants ─────────────────────────────────────────────────────────────

let RESULT_PATH = "/tmp/mic_indicator_result"

private let PILL_SIZE: CGFloat = 48
private let MENU_WIDTH: CGFloat = 200
private let MENU_HEIGHT: CGFloat = 44

// ─── Rounded button helper ─────────────────────────────────────────────────

private func makeButton(title: String, color: CGColor, target: AnyObject, action: Selector) -> NSButton {
    let b = NSButton(frame: .zero)
    b.title = title
    b.bezelStyle = .rounded
    b.isBordered = false
    b.wantsLayer = true
    b.layer?.cornerRadius = 8
    b.layer?.backgroundColor = color
    b.font = NSFont.systemFont(ofSize: 14, weight: .semibold)
    b.contentTintColor = .white
    b.target = target
    b.action = action
    return b
}

// ─── Indicator ─────────────────────────────────────────────────────────────

class IndicatorWindow {
    private var panel: NSPanel?
    private var confirmBtn: NSButton?
    private var cancelBtn: NSButton?
    private var stack: NSStackView?

    // ── show / hide ──

    func showRecording(atX x: CGFloat, y: CGFloat) {
        unlink(RESULT_PATH)
        DispatchQueue.main.async { self._showPill(atX: x, y: y) }
    }

    func showMenu(atX x: CGFloat, y: CGFloat) {
        DispatchQueue.main.async { self._showMenu(atX: x, y: y) }
    }

    func hide() {
        DispatchQueue.main.async { self._doHide() }
    }

    // ── pill mode ──

    private func _showPill(atX x: CGFloat, y: CGFloat) {
        if panel == nil { _createPanel(width: PILL_SIZE, height: PILL_SIZE, ignoresMouse: true) }
        guard let p = panel else { return }
        _clearContent()

        p.setContentSize(NSSize(width: PILL_SIZE, height: PILL_SIZE))

        let root = NSView(frame: NSRect(x: 0, y: 0, width: PILL_SIZE, height: PILL_SIZE))
        root.wantsLayer = true

        let bg = CALayer()
        bg.frame = root.bounds.insetBy(dx: 2, dy: 2)
        bg.cornerRadius = bg.frame.height / 2
        bg.backgroundColor = CGColor(red: 0, green: 0, blue: 0, alpha: 0.55)
        bg.shadowColor = CGColor(red: 0, green: 0, blue: 0, alpha: 0.3)
        bg.shadowOpacity = 1
        bg.shadowOffset = .zero
        bg.shadowRadius = 8
        root.layer?.addSublayer(bg)

        let img = NSImageView(frame: NSRect(x: 0, y: 0, width: PILL_SIZE, height: PILL_SIZE))
        let cfg = NSImage.SymbolConfiguration(pointSize: 22, weight: .semibold)
        if let icon = NSImage(systemSymbolName: "mic.fill", accessibilityDescription: "Recording")?.withSymbolConfiguration(cfg) {
            img.image = icon
        }
        img.contentTintColor = .white
        img.imageScaling = .scaleProportionallyUpOrDown
        root.addSubview(img)

        p.contentView = root
        _position(centeredOn: x, y: y, width: PILL_SIZE, height: PILL_SIZE)
        _fadeIn()
        _startBreathing()
    }

    // ── menu mode ──

    private func _showMenu(atX x: CGFloat, y: CGFloat) {
        if panel == nil { _createPanel(width: MENU_WIDTH, height: MENU_HEIGHT, ignoresMouse: false) }
        guard let p = panel else { return }
        _clearContent()
        _stopBreathing()

        p.setContentSize(NSSize(width: MENU_WIDTH, height: MENU_HEIGHT))

        let root = NSView(frame: NSRect(x: 0, y: 0, width: MENU_WIDTH, height: MENU_HEIGHT))
        root.wantsLayer = true

        let bg = CALayer()
        bg.frame = root.bounds
        bg.cornerRadius = MENU_HEIGHT / 2
        bg.backgroundColor = CGColor(red: 0, green: 0, blue: 0, alpha: 0.7)
        bg.shadowColor = CGColor(red: 0, green: 0, blue: 0, alpha: 0.3)
        bg.shadowOpacity = 1
        bg.shadowOffset = .zero
        bg.shadowRadius = 10
        root.layer?.addSublayer(bg)

        let sv = NSStackView(frame: NSRect(x: 12, y: 10, width: MENU_WIDTH - 24, height: MENU_HEIGHT - 20))
        sv.orientation = .horizontal
        sv.spacing = 12
        sv.alignment = .centerY
        sv.distribution = .fillEqually

        let confirm = makeButton(
            title: "✓  完成",
            color: CGColor(red: 0.22, green: 0.45, blue: 0.95, alpha: 1),
            target: self,
            action: #selector(_onConfirm)
        )
        confirmBtn = confirm

        let cancel = makeButton(
            title: "✕  取消",
            color: CGColor(red: 0.5, green: 0.5, blue: 0.5, alpha: 0.9),
            target: self,
            action: #selector(_onCancel)
        )
        cancelBtn = cancel

        sv.addArrangedSubview(confirm)
        sv.addArrangedSubview(cancel)
        sv.setHuggingPriority(.defaultHigh, for: .horizontal)
        root.addSubview(sv)
        stack = sv

        p.contentView = root
        _position(centeredOn: x, y: y, width: MENU_WIDTH, height: MENU_HEIGHT)
        p.alphaValue = 0
        p.setIsVisible(true)
        p.orderFront(nil)

        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.15
            ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
            p.animator().alphaValue = 1
        }
    }

    // ── button handlers ──

    @objc private func _onConfirm() {
        try? "confirm".write(toFile: RESULT_PATH, atomically: true, encoding: .utf8)
        _doHide()
    }

    @objc private func _onCancel() {
        try? "cancel".write(toFile: RESULT_PATH, atomically: true, encoding: .utf8)
        _doHide()
    }

    // ── panel creation ──

    private func _createPanel(width: CGFloat, height: CGFloat, ignoresMouse: Bool) {
        let p = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: width, height: height),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        p.isOpaque = false
        p.backgroundColor = .clear
        p.hasShadow = false
        p.level = .floating
        p.ignoresMouseEvents = ignoresMouse
        p.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .transient]
        p.isFloatingPanel = true
        p.hidesOnDeactivate = false
        p.worksWhenModal = true
        self.panel = p
    }

    private func _clearContent() {
        panel?.contentView = NSView(frame: .zero)
        confirmBtn = nil
        cancelBtn = nil
        stack = nil
    }

    // ── positioning ──

    private func _position(centeredOn x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) {
        guard let screen = NSScreen.main?.visibleFrame else { return }
        var wx = x - width / 2
        var wy = y - height - 8
        wy = max(screen.minY + 4, min(wy, screen.maxY - height - 4))
        wx = max(screen.minX + 4, min(wx, screen.maxX - width - 4))
        panel?.setFrameOrigin(NSPoint(x: wx, y: wy))
    }

    // ── fade ──

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
        _stopBreathing()
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

    // ── breathing ──

    private var _breathing = false
    private var _scaledUp = true
    private var _currentScale: CGFloat = 1

    private func _startBreathing() {
        guard let p = panel, p.isVisible, !_breathing else { return }
        _currentScale = 1
        _breathing = true
        _breath()
    }

    private func _stopBreathing() { _breathing = false }

    private func _breath() {
        guard _breathing, let p = panel, p.isVisible else { _breathing = false; return }
        let targetScale: CGFloat = _scaledUp ? 0.88 : 1.0
        let targetOpacity: CGFloat = _scaledUp ? 0.7 : 1.0
        _scaledUp.toggle()
        let dur = 0.7

        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = dur
            ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            p.animator().alphaValue = targetOpacity
        }

        if let layer = p.contentView?.layer {
            let anim = CABasicAnimation(keyPath: "transform.scale")
            anim.fromValue = _currentScale
            anim.toValue = targetScale
            anim.duration = dur
            anim.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            layer.transform = CATransform3DMakeScale(targetScale, targetScale, 1)
            layer.add(anim, forKey: "breathingScale")
            _currentScale = targetScale
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
        let parts = cmd.split(separator: " ", maxSplits: 3).map(String.init)
        guard let action = parts.first else { return }
        switch action {
        case "recording":
            if parts.count >= 3, let x = Double(parts[1]), let y = Double(parts[2]) {
                indicator.showRecording(atX: CGFloat(x), y: CGFloat(y))
            }
        case "menu":
            if parts.count >= 3, let x = Double(parts[1]), let y = Double(parts[2]) {
                indicator.showMenu(atX: CGFloat(x), y: CGFloat(y))
            }
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
