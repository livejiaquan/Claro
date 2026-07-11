import Cocoa
import AVFoundation
import Darwin

let SOUND_START = "/System/Library/Sounds/Ping.aiff"
let SOUND_SUCCESS = "/System/Library/Sounds/Pop.aiff"
let SOUND_ERROR = "/System/Library/Sounds/Basso.aiff"
let CLARO_DIR = NSHomeDirectory() + "/.claro"
let HISTORY_PATH = CLARO_DIR + "/history.jsonl"

private let PILL_W: CGFloat = 150
private let PILL_H: CGFloat = 38
private let PILL_CORNER: CGFloat = 19
private let NUM_BARS = 9
private let BAR_W: CGFloat = 3
private let BAR_GAP: CGFloat = 5
private let DOT_SIZE: CGFloat = 6

private var soundPlayers: [AVAudioPlayer] = []

// Helpers

private func playSound(_ path: String) {
    let url = URL(fileURLWithPath: path)
    guard let player = try? AVAudioPlayer(contentsOf: url) else { return }
    player.volume = 0.35
    player.play()
    soundPlayers.removeAll { !$0.isPlaying }
    soundPlayers.append(player)
}

private enum IndicatorState {
    case hidden
    case recording
    case handsfree
    case processing
    case success
    case cancelled
    case error
}

// Indicator

class IndicatorWindow: NSObject {
    private var panel: NSPanel?
    private var state: IndicatorState = .hidden

    private var rootView: NSView?
    private var contentView: NSView?
    private var backgroundLayer: CALayer?
    private var barLayers: [CALayer] = []
    private var processingDots: [CALayer] = []
    private var handsfreeDot: CALayer?

    private var frameTimer: Timer?
    private var autoHideItem: DispatchWorkItem?
    private var visibilityToken = 0

    private var targetLevel: CGFloat = 0
    private var displayLevel: CGFloat = 0

    func showRecording() {
        DispatchQueue.main.async {
            self.beginState(.recording)
            self.targetLevel = 0
            self.displayLevel = 0
            self.buildWaveform(handsfree: false)
            self.showPanel()
            self.startFrameTimer()
            playSound(SOUND_START)
        }
    }

    func showHandsfree() {
        DispatchQueue.main.async {
            self.beginState(.handsfree)
            self.buildWaveform(handsfree: true)
            self.showPanel()
            self.startFrameTimer()
        }
    }

    func updateLevel(_ level: Float) {
        DispatchQueue.main.async {
            self.targetLevel = min(1.0, CGFloat(level) * 9.0)
        }
    }

    func showProcessing() {
        DispatchQueue.main.async {
            self.beginState(.processing)
            self.buildProcessing()
            self.showPanel()
            self.startFrameTimer()
        }
    }

    func showSuccess() {
        DispatchQueue.main.async {
            self.beginState(.success)
            self.stopFrameTimer()
            self.buildIcon(systemName: "checkmark.circle", background: Self.defaultBackground)
            self.showPanel()
            playSound(SOUND_SUCCESS)
            self.scheduleHide(after: 0.45, expected: .success)
        }
    }

    func showCancelled() {
        DispatchQueue.main.async {
            self.beginState(.cancelled)
            self.stopFrameTimer()
            if self.panel == nil || self.panel?.isVisible != true {
                self.buildEmpty(background: Self.defaultBackground)
            } else {
                self.backgroundLayer?.backgroundColor = Self.defaultBackground
            }
            self.showPanel()
            playSound(SOUND_ERROR)
            self.runCancelShake()
        }
    }

    func showError() {
        DispatchQueue.main.async {
            self.beginState(.error)
            self.stopFrameTimer()
            self.buildIcon(systemName: "exclamationmark.circle", background: Self.errorBackground)
            self.showPanel()
            playSound(SOUND_ERROR)
            self.scheduleHide(after: 0.6, expected: .error)
        }
    }

    func hide() {
        DispatchQueue.main.async {
            self.transitionToHidden()
        }
    }

    private static var defaultBackground: CGColor {
        CGColor(red: 0, green: 0, blue: 0, alpha: 0.75)
    }

    private static var errorBackground: CGColor {
        CGColor(red: 0.45, green: 0.08, blue: 0.08, alpha: 0.82)
    }

    private func beginState(_ newState: IndicatorState) {
        autoHideItem?.cancel()
        autoHideItem = nil
        visibilityToken += 1
        state = newState
    }

    private func transitionToHidden() {
        autoHideItem?.cancel()
        autoHideItem = nil
        visibilityToken += 1
        state = .hidden
        stopFrameTimer()
        animateHide(token: visibilityToken)
    }

    private func scheduleHide(after delay: TimeInterval, expected: IndicatorState) {
        autoHideItem?.cancel()
        let item = DispatchWorkItem { [weak self] in
            guard let self = self, self.state == expected else { return }
            self.transitionToHidden()
        }
        autoHideItem = item
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: item)
    }

    private func ensurePanel() {
        guard panel == nil else { return }
        let p = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: PILL_W, height: PILL_H),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        p.isOpaque = false
        p.backgroundColor = .clear
        p.hasShadow = false
        p.level = .floating
        p.ignoresMouseEvents = true
        p.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .transient]
        p.isFloatingPanel = true
        p.hidesOnDeactivate = false
        p.worksWhenModal = true
        panel = p
    }

    private func prepareRoot(background: CGColor) {
        ensurePanel()
        guard let p = panel else { return }

        p.setContentSize(NSSize(width: PILL_W, height: PILL_H))

        let root = NSView(frame: NSRect(x: 0, y: 0, width: PILL_W, height: PILL_H))
        root.wantsLayer = true
        if let layer = root.layer {
            layer.cornerRadius = PILL_CORNER
            layer.backgroundColor = background
            layer.borderWidth = 1
            layer.borderColor = CGColor(red: 1, green: 1, blue: 1, alpha: 0.12)
            layer.shadowColor = CGColor(red: 0, green: 0, blue: 0, alpha: 1)
            layer.shadowOpacity = 0.3
            layer.shadowOffset = .zero
            layer.shadowRadius = 10
            layer.shadowPath = CGPath(
                roundedRect: root.bounds,
                cornerWidth: PILL_CORNER,
                cornerHeight: PILL_CORNER,
                transform: nil
            )
            layer.masksToBounds = false
        }

        let content = NSView(frame: root.bounds)
        content.wantsLayer = true
        root.addSubview(content)

        p.contentView = root
        rootView = root
        contentView = content
        backgroundLayer = root.layer
        barLayers.removeAll()
        processingDots.removeAll()
        handsfreeDot = nil
    }

    private func buildEmpty(background: CGColor) {
        prepareRoot(background: background)
    }

    private func buildWaveform(handsfree: Bool) {
        prepareRoot(background: Self.defaultBackground)
        guard let contentLayer = contentView?.layer else { return }

        let totalW = CGFloat(NUM_BARS) * BAR_W + CGFloat(NUM_BARS - 1) * BAR_GAP
        let originX = (PILL_W - totalW) / 2

        for i in 0 ..< NUM_BARS {
            let layer = CALayer()
            layer.backgroundColor = CGColor(red: 1, green: 1, blue: 1, alpha: 0.9)
            layer.cornerRadius = 1.5
            let x = originX + CGFloat(i) * (BAR_W + BAR_GAP)
            layer.frame = NSRect(x: x, y: (PILL_H - 3) / 2, width: BAR_W, height: 3)
            contentLayer.addSublayer(layer)
            barLayers.append(layer)
        }

        if handsfree {
            let dot = CALayer()
            dot.backgroundColor = CGColor(red: 1, green: 1, blue: 1, alpha: 1)
            dot.cornerRadius = DOT_SIZE / 2
            dot.frame = NSRect(x: 14 - DOT_SIZE / 2, y: (PILL_H - DOT_SIZE) / 2, width: DOT_SIZE, height: DOT_SIZE)
            contentLayer.addSublayer(dot)
            handsfreeDot = dot
        }
    }

    private func buildProcessing() {
        prepareRoot(background: Self.defaultBackground)
        guard let contentLayer = contentView?.layer else { return }

        let gap: CGFloat = 10
        let totalW = 3 * DOT_SIZE + 2 * gap
        let originX = (PILL_W - totalW) / 2
        for i in 0 ..< 3 {
            let dot = CALayer()
            dot.backgroundColor = CGColor(red: 1, green: 1, blue: 1, alpha: 1)
            dot.cornerRadius = DOT_SIZE / 2
            dot.opacity = 0.35
            dot.frame = NSRect(
                x: originX + CGFloat(i) * (DOT_SIZE + gap),
                y: (PILL_H - DOT_SIZE) / 2,
                width: DOT_SIZE,
                height: DOT_SIZE
            )
            contentLayer.addSublayer(dot)
            processingDots.append(dot)
        }
    }

    private func buildIcon(systemName: String, background: CGColor) {
        prepareRoot(background: background)
        guard let content = contentView else { return }

        let iconSize: CGFloat = 24
        let imageView = NSImageView(frame: NSRect(
            x: (PILL_W - iconSize) / 2,
            y: (PILL_H - iconSize) / 2,
            width: iconSize,
            height: iconSize
        ))
        if let image = NSImage(systemSymbolName: systemName, accessibilityDescription: nil) {
            let config = NSImage.SymbolConfiguration(pointSize: 18, weight: .semibold)
            imageView.image = image.withSymbolConfiguration(config)
        }
        imageView.contentTintColor = .white
        imageView.imageScaling = .scaleProportionallyUpOrDown
        content.addSubview(imageView)
    }

    private func positionBottom() {
        guard let p = panel, let frame = NSScreen.main?.visibleFrame else { return }
        let x = frame.minX + (frame.width - PILL_W) / 2
        let y = frame.minY + 16
        p.setFrame(NSRect(x: x, y: y, width: PILL_W, height: PILL_H), display: true)
    }

    private func showPanel() {
        ensurePanel()
        guard let p = panel else { return }

        positionBottom()
        rootView?.layer?.removeAllAnimations()
        contentView?.layer?.removeAllAnimations()

        let shouldAnimate = !p.isVisible || p.alphaValue < 0.99
        p.orderFrontRegardless()

        if shouldAnimate {
            p.alphaValue = 0
            rootView?.layer?.transform = CATransform3DMakeTranslation(0, -8, 0)
            p.setIsVisible(true)

            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.25
                context.timingFunction = CAMediaTimingFunction(name: .easeOut)
                p.animator().alphaValue = 1
            }, completionHandler: nil)

            if let layer = rootView?.layer {
                let spring = CASpringAnimation(keyPath: "transform.translation.y")
                spring.fromValue = -8
                spring.toValue = 0
                spring.mass = 1
                spring.stiffness = 220
                spring.damping = 22
                spring.initialVelocity = 0
                spring.duration = 0.25
                layer.transform = CATransform3DIdentity
                layer.add(spring, forKey: "entranceY")
            }
        } else {
            p.alphaValue = 1
            p.setIsVisible(true)
            rootView?.layer?.transform = CATransform3DIdentity
        }
    }

    private func animateHide(token: Int) {
        guard let p = panel, p.isVisible else {
            clearVisualReferences()
            return
        }

        if let layer = rootView?.layer {
            layer.removeAllAnimations()
            let sink = CABasicAnimation(keyPath: "transform.translation.y")
            sink.fromValue = 0
            sink.toValue = -8
            sink.duration = 0.18
            sink.timingFunction = CAMediaTimingFunction(name: .easeIn)
            layer.transform = CATransform3DMakeTranslation(0, -8, 0)
            layer.add(sink, forKey: "exitY")
        }

        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.18
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            p.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            guard let self = self, self.visibilityToken == token else { return }
            p.setIsVisible(false)
            p.alphaValue = 1
            p.contentView = NSView(frame: .zero)
            self.clearVisualReferences()
        })
    }

    private func clearVisualReferences() {
        rootView = nil
        contentView = nil
        backgroundLayer = nil
        barLayers.removeAll()
        processingDots.removeAll()
        handsfreeDot = nil
    }

    private func runCancelShake() {
        if let layer = rootView?.layer {
            layer.removeAnimation(forKey: "cancelShake")
            let shake = CAKeyframeAnimation(keyPath: "transform.translation.x")
            shake.values = [0, -6, 5, -3, 2, 0]
            shake.keyTimes = [0, 0.2, 0.4, 0.6, 0.8, 1]
            shake.duration = 0.22
            shake.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            layer.add(shake, forKey: "cancelShake")
        }
        scheduleHide(after: 0.22, expected: .cancelled)
    }

    private func startFrameTimer() {
        frameTimer?.invalidate()
        let timer = Timer(timeInterval: 1.0 / 120.0, target: self, selector: #selector(frameTick), userInfo: nil, repeats: true)
        RunLoop.main.add(timer, forMode: .common)
        frameTimer = timer
    }

    private func stopFrameTimer() {
        frameTimer?.invalidate()
        frameTimer = nil
    }

    @objc private func frameTick() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        switch state {
        case .recording, .handsfree:
            updateWaveformFrame()
        case .processing:
            updateProcessingFrame()
        default:
            break
        }
        CATransaction.commit()
    }

    private func updateWaveformFrame() {
        let speed: CGFloat = (targetLevel > displayLevel) ? 0.45 : 0.10
        displayLevel += (targetLevel - displayLevel) * speed

        let t = CACurrentMediaTime()
        let totalW = CGFloat(NUM_BARS) * BAR_W + CGFloat(NUM_BARS - 1) * BAR_GAP
        let originX = (PILL_W - totalW) / 2

        for i in 0 ..< NUM_BARS {
            let weight = 1.0 - CGFloat(abs(i - 4)) / 4.0 * 0.55
            let phase = 0.75 + 0.25 * CGFloat(sin(t * 5.0 + Double(i) * 0.9))
            let idle = 0.8 * CGFloat(sin(t * 2.0 + Double(i)))
            let h = max(3.0, 3.0 + idle + 23.0 * displayLevel * weight * phase)
            let x = originX + CGFloat(i) * (BAR_W + BAR_GAP)
            barLayers[i].frame = NSRect(x: x, y: (PILL_H - h) / 2.0, width: BAR_W, height: h)
        }

        if let dot = handsfreeDot {
            let alpha = 0.4 + 0.6 * (0.5 + 0.5 * CGFloat(sin(t * 2.4)))
            dot.opacity = Float(alpha)
        }
    }

    private func updateProcessingFrame() {
        let t = CACurrentMediaTime()
        for (i, dot) in processingDots.enumerated() {
            let wave = max(0.0, sin(t * 3.0 - Double(i) * 0.55))
            dot.opacity = Float(0.35 + 0.65 * wave)
        }
    }
}

// Menu Bar

class MenuBarController: NSObject, NSMenuDelegate {
    private let statusItem: NSStatusItem
    private let menu = NSMenu()
    private let statusLine = NSMenuItem(title: "Claro — 待命", action: nil, keyEquivalent: "")
    private let statsLine = NSMenuItem(title: "今日 0 次・0 字", action: nil, keyEquivalent: "")
    private let recentMenu = NSMenu()
    private var currentStateText = "待命"

    override init() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        super.init()

        if let button = statusItem.button {
            if let image = NSImage(systemSymbolName: "mic.fill", accessibilityDescription: "Claro") {
                image.isTemplate = true
                button.image = image
            }
            button.imagePosition = .imageOnly
        }

        statusLine.isEnabled = false
        statsLine.isEnabled = false
        menu.delegate = self
        menu.addItem(statusLine)
        menu.addItem(.separator())
        menu.addItem(statsLine)

        let recentItem = NSMenuItem(title: "最近聽寫", action: nil, keyEquivalent: "")
        recentItem.submenu = recentMenu
        menu.addItem(recentItem)

        let openItem = NSMenuItem(title: "開啟 Claro", action: #selector(openClaro(_:)), keyEquivalent: "")
        openItem.target = self
        menu.addItem(openItem)

        let historyItem = NSMenuItem(title: "開啟歷史紀錄", action: #selector(openHistory(_:)), keyEquivalent: "")
        historyItem.target = self
        menu.addItem(historyItem)

        menu.addItem(.separator())

        let quitItem = NSMenuItem(title: "結束 Claro", action: #selector(quitClaro(_:)), keyEquivalent: "")
        quitItem.target = self
        menu.addItem(quitItem)

        statusItem.menu = menu
        refreshMenu()
    }

    func setState(_ text: String) {
        DispatchQueue.main.async {
            self.currentStateText = text
            self.updateStatusLine()
        }
    }

    func menuWillOpen(_ menu: NSMenu) {
        refreshMenu()
    }

    private func refreshMenu() {
        updateStatusLine()
        let history = readHistory()
        statsLine.title = "今日 \(history.todayCount) 次・\(history.todayChars) 字"
        rebuildRecentMenu(texts: history.recentTexts)
    }

    private func updateStatusLine() {
        statusLine.title = "Claro — \(currentStateText)"
    }

    private func readHistory() -> (todayCount: Int, todayChars: Int, recentTexts: [String]) {
        guard let contents = try? String(contentsOfFile: HISTORY_PATH, encoding: .utf8) else {
            return (0, 0, [])
        }

        let formatter = DateFormatter()
        formatter.calendar = Calendar.current
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyy-MM-dd"
        let todayPrefix = formatter.string(from: Date())

        var todayCount = 0
        var todayChars = 0
        var pastedTexts: [String] = []

        for lineSub in contents.split(separator: "\n").suffix(200) {
            let line = String(lineSub)
            guard let data = line.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data),
                  let entry = obj as? [String: Any],
                  entry["status"] as? String == "pasted" else {
                continue
            }
            let text = entry["text"] as? String ?? ""
            if let ts = entry["ts"] as? String, ts.hasPrefix(todayPrefix) {
                todayCount += 1
                todayChars += text.count
            }
            if !text.isEmpty {
                pastedTexts.append(text)
            }
        }

        return (todayCount, todayChars, Array(pastedTexts.suffix(5).reversed()))
    }

    private func rebuildRecentMenu(texts: [String]) {
        recentMenu.removeAllItems()
        if texts.isEmpty {
            let empty = NSMenuItem(title: "尚無紀錄", action: nil, keyEquivalent: "")
            empty.isEnabled = false
            recentMenu.addItem(empty)
            return
        }
        for text in texts {
            let item = NSMenuItem(title: truncated(text, limit: 30), action: #selector(copyRecent(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = text
            recentMenu.addItem(item)
        }
    }

    private func truncated(_ text: String, limit: Int) -> String {
        if text.count <= limit {
            return text
        }
        return String(text.prefix(limit))
    }

    @objc private func copyRecent(_ sender: NSMenuItem) {
        guard let text = sender.representedObject as? String else { return }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
    }

    @objc private func openHistory(_ sender: NSMenuItem) {
        NSWorkspace.shared.open(URL(fileURLWithPath: HISTORY_PATH))
    }

    @objc private func openClaro(_ sender: NSMenuItem) {
        let executable = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath()
        let appURL = executable
            .deletingLastPathComponent() // MacOS
            .deletingLastPathComponent() // Contents
            .deletingLastPathComponent() // Claro.app
        guard appURL.pathExtension == "app" else { return }
        let config = NSWorkspace.OpenConfiguration()
        config.activates = true
        NSWorkspace.shared.openApplication(at: appURL, configuration: config, completionHandler: nil)
    }

    @objc private func quitClaro(_ sender: NSMenuItem) {
        if CommandLine.arguments.count > 1, let pid = Int32(CommandLine.arguments[1]) {
            Darwin.kill(pid, SIGTERM)
        }
        NSApplication.shared.terminate(nil)
    }
}

// Socket Server

class SocketServer {
    private let path: String
    private var sock: Int32 = -1

    init(path: String) { self.path = path }

    func start(indicator: IndicatorWindow, menu: MenuBarController) -> Bool {
        let claroDir = CLARO_DIR
        try? FileManager.default.createDirectory(
            atPath: claroDir,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        // createDirectory 的 attributes 不影響既有目錄，明確補一次
        try? FileManager.default.setAttributes(
            [.posixPermissions: 0o700], ofItemAtPath: claroDir
        )
        unlink(path)
        sock = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard sock >= 0 else { return false }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = path.utf8CString
        let maxPath = MemoryLayout.size(ofValue: addr.sun_path)
        guard bytes.count - 1 < maxPath else { close(sock); sock = -1; return false }

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
        guard ok == 0 else { close(sock); sock = -1; return false }

        chmod(path, 0o600)
        guard listen(sock, 5) == 0 else {
            close(sock)
            sock = -1
            unlink(path)
            return false
        }

        DispatchQueue.global(qos: .background).async { [weak self] in
            self?.acceptLoop(indicator: indicator, menu: menu)
        }
        return true
    }

    private func acceptLoop(indicator: IndicatorWindow, menu: MenuBarController) {
        while sock >= 0 {
            let client = accept(sock, nil, nil)
            guard client >= 0 else { continue }
            var buf = [UInt8](repeating: 0, count: 512)
            let n = read(client, &buf, 511)
            var cmd = ""
            if n > 0 { buf[n] = 0; cmd = String(cString: buf).trimmingCharacters(in: .whitespacesAndNewlines) }
            close(client)
            handle(cmd, indicator: indicator, menu: menu)
        }
    }

    private func handle(_ cmd: String, indicator: IndicatorWindow, menu: MenuBarController) {
        let parts = cmd.split(separator: " ", maxSplits: 1).map(String.init)
        guard let action = parts.first else { return }
        switch action {
        case "recording":
            menu.setState("錄音中")
            indicator.showRecording()
        case "handsfree":
            menu.setState("錄音中")
            indicator.showHandsfree()
        case "level":
            if parts.count >= 2, let v = Float(parts[1]) {
                indicator.updateLevel(v)
            }
        case "processing":
            menu.setState("處理中")
            indicator.showProcessing()
        case "success":
            menu.setState("待命")
            indicator.showSuccess()
        case "error":
            menu.setState("待命")
            indicator.showError()
        case "cancel":
            menu.setState("待命")
            indicator.showCancelled()
        case "hide":
            menu.setState("待命")
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

final class ParentWatcher {
    private var source: DispatchSourceProcess?

    init?(pid: pid_t) {
        guard pid > 1, kill(pid, 0) == 0 || errno != ESRCH else { return nil }
        let source = DispatchSource.makeProcessSource(identifier: pid, eventMask: .exit, queue: .main)
        source.setEventHandler {
            NSApplication.shared.terminate(nil)
        }
        source.resume()
        self.source = source
    }
}

// Entry

try? FileManager.default.createDirectory(
    atPath: CLARO_DIR,
    withIntermediateDirectories: true,
    attributes: [.posixPermissions: 0o700]
)
let lockFD = Darwin.open(CLARO_DIR + "/indicator.lock", O_CREAT | O_RDWR, 0o600)
guard lockFD >= 0, flock(lockFD, LOCK_EX | LOCK_NB) == 0 else {
    exit(0)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let indicator = IndicatorWindow()
let menuBar = MenuBarController()
let server = SocketServer(path: CLARO_DIR + "/indicator.sock")
guard server.start(indicator: indicator, menu: menuBar) else {
    exit(1)
}
let parentWatcher: ParentWatcher? = {
    guard CommandLine.arguments.count > 1, let pid = pid_t(CommandLine.arguments[1]) else { return nil }
    return ParentWatcher(pid: pid)
}()
atexit { server.stop() }
app.run()
