import AVFoundation
import IsctCore
import SwiftUI
import UIKit

/// Live Matrix scanning: AVFoundation hands each NV12 frame to Core's camera
/// session (the same driver as the desktop camera and photo import). As in
/// the system camera the live preview fills the screen and a tap focuses
/// there; a card-shaped guide shows where to hold the card, and the outline
/// Core last fitted is drawn over it. The grid fills in as Core confirms cells.
struct MatrixCameraScreen: View {
    let session: MatrixCameraSession
    /// The final cells, or nil when the scan was cancelled or failed.
    let done: ([MatrixCell]?) -> Void
    @State private var camera: MatrixCameraController?
    @State private var state: MatrixCameraState?
    @State private var error: String?
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            if let camera { FrameView(controller: camera).ignoresSafeArea() }
            VStack(spacing: 0) {
                HStack {
                    Button { finish(nil) } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 17, weight: .semibold))
                            .frame(width: 44, height: 44)
                            .background(.ultraThinMaterial, in: Circle())
                    }
                    .accessibilityLabel(t("Cancel"))
                    Spacer()
                }
                .padding(.horizontal, 16)
                Spacer(minLength: 16)
                CardGuide().padding(.horizontal, 24).allowsHitTesting(false)
                Spacer(minLength: 16)
                VStack(spacing: 10) {
                    LiveGrid(cells: state?.cells ?? [])
                    Text(error ?? status)
                        .font(.footnote)
                        .foregroundStyle(error == nil ? Color.white.opacity(0.85) : Theme.danger)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: .infinity)
                }
                .padding(14)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
            }
            .foregroundStyle(.white)
        }
        .environment(\.colorScheme, .dark)
        .statusBarHidden()
        .task { await start() }
        .onDisappear { camera?.stop() }
        .onChange(of: scenePhase) { _, phase in
            // As on desktop: leaving the app stops acquisition for good.
            if phase != .active, state?.phase != .complete {
                fail(t("Camera scan stopped while the app was in the background. Open it again to continue."))
            }
        }
    }

    private var status: String {
        guard let state, state.phase != .complete, state.readyCells > 0 else {
            return t("Keep the Matrix framed and steady. Recognition finishes after all cells are confirmed, or within ten seconds.")
        }
        return t("Recognizing Matrix… {{count}}/70 cells confirmed.", ["count": state.readyCells])
    }

    private func start() async {
        guard await AVCaptureDevice.requestAccess(for: .video) else {
            fail(t("Camera access was denied. Allow camera access in system settings, then try again."))
            return
        }
        let controller = MatrixCameraController(session: session) { update in
            switch update {
            case let .state(next):
                state = next
                if next.phase == .complete { finish(next.cells) }
            case let .failed(message):
                fail(message)
            }
        }
        camera = controller
        controller.start()
    }

    private func fail(_ message: String) {
        camera?.stop()
        error = message
    }

    private func finish(_ cells: [MatrixCell]?) {
        camera?.stop()
        camera = nil
        done(cells)
    }
}

/// Where to hold the card: an ID-1 card's outline, centred.
private struct CardGuide: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 14, style: .continuous)
            .strokeBorder(Color.white.opacity(0.7), lineWidth: 2)
            .aspectRatio(85.6 / 53.98, contentMode: .fit)
            .frame(maxWidth: 480)
    }
}

/// The 7×10 letters as Core confirms them; unconfirmed reads stay faint.
private struct LiveGrid: View {
    let cells: [MatrixCell]

    var body: some View {
        Grid(horizontalSpacing: 2, verticalSpacing: 2) {
            ForEach(0..<7, id: \.self) { row in
                GridRow {
                    ForEach(0..<10, id: \.self) { column in
                        let cell = cells.indices.contains(row * 10 + column) ? cells[row * 10 + column] : nil
                        Text(cell?.text ?? "")
                            .font(.caption.monospaced().weight(.semibold))
                            .foregroundStyle(cell?.stable == true ? Color.black : Color.white.opacity(0.6))
                            .frame(maxWidth: .infinity, minHeight: 20)
                            .background(cell?.stable == true ? Theme.okSoft : Color.white.opacity(0.12),
                                        in: RoundedRectangle(cornerRadius: 4))
                    }
                }
            }
        }
        .privacySensitive()
    }
}

/// Shows the live preview and the outline.
private struct FrameView: UIViewRepresentable {
    let controller: MatrixCameraController

    func makeUIView(context: Context) -> FrameLayerView {
        let view = FrameLayerView(session: controller.capture) { [controller] point in
            controller.focus(at: point)
        }
        controller.display = view
        return view
    }

    func updateUIView(_ view: FrameLayerView, context: Context) {}
}

final class FrameLayerView: UIView {
    private let video: AVCaptureVideoPreviewLayer
    private let outline = CAShapeLayer()
    private let focusBox = CAShapeLayer()
    private var imageSize = CGSize(width: 1, height: 1)
    private var bounds4: [MatrixPoint] = []
    /// A tap's point in the capture device's coordinates.
    private let focus: (CGPoint) -> Void

    init(session: AVCaptureSession, focus: @escaping (CGPoint) -> Void) {
        video = AVCaptureVideoPreviewLayer(session: session)
        self.focus = focus
        super.init(frame: .zero)
        video.videoGravity = .resizeAspectFill
        outline.fillColor = nil
        outline.strokeColor = UIColor(red: 1, green: 0.27, blue: 0.27, alpha: 1).cgColor
        outline.lineWidth = 3
        focusBox.fillColor = nil
        focusBox.strokeColor = UIColor.systemYellow.cgColor
        focusBox.lineWidth = 1.5
        focusBox.opacity = 0
        layer.addSublayer(video)
        layer.addSublayer(outline)
        layer.addSublayer(focusBox)
        addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tapped)))
    }

    /// Focus where tapped, with the system camera's yellow square.
    @objc private func tapped(_ tap: UITapGestureRecognizer) {
        let point = tap.location(in: self)
        focus(video.captureDevicePointConverted(fromLayerPoint: point))
        focusBox.path = UIBezierPath(rect: CGRect(x: point.x - 40, y: point.y - 40, width: 80, height: 80)).cgPath
        let fade = CAKeyframeAnimation(keyPath: "opacity")
        fade.values = [1, 1, 0]
        fade.keyTimes = [0, 0.7, 1]
        fade.duration = 1.2
        focusBox.add(fade, forKey: "focus")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func layoutSubviews() {
        super.layoutSubviews()
        video.frame = bounds
        outline.frame = bounds
        drawOutline()
    }

    /// The preview turns with the frames Core reads, so the outline's
    /// normalized points map onto the same upright picture.
    func rotate(_ angle: CGFloat) {
        guard let connection = video.connection, connection.isVideoRotationAngleSupported(angle) else { return }
        connection.videoRotationAngle = angle
    }

    /// The outline Core fitted on the frame it last analysed.
    func show(size: CGSize, outline points: [MatrixPoint]) {
        imageSize = size
        bounds4 = points
        drawOutline()
    }

    private func drawOutline() {
        guard bounds4.count == 4 else {
            outline.path = nil
            return
        }
        // The preview fills the view, so the frame overflows it on one axis.
        let scale = max(bounds.width / imageSize.width, bounds.height / imageSize.height)
        let size = CGSize(width: imageSize.width * scale, height: imageSize.height * scale)
        let rect = CGRect(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2,
                          width: size.width, height: size.height)
        let path = UIBezierPath()
        for (index, point) in bounds4.enumerated() {
            let p = CGPoint(x: rect.minX + point.x * rect.width, y: rect.minY + point.y * rect.height)
            if index == 0 { path.move(to: p) } else { path.addLine(to: p) }
        }
        path.close()
        outline.path = path.cgPath
    }
}

/// Owns the capture session and one in-flight frame. Frames arriving while
/// Core analyses the previous one are dropped, as on desktop.
final class MatrixCameraController: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
    enum Update {
        case state(MatrixCameraState)
        case failed(String)
    }

    private let scan: MatrixCameraSession
    private let update: @MainActor (Update) -> Void
    let capture = AVCaptureSession()
    private let output = AVCaptureVideoDataOutput()
    private let queue = DispatchQueue(label: "app.isctcompanion.matrix-camera")
    private var rotation: AVCaptureDevice.RotationCoordinator?
    private var rotationObservation: NSKeyValueObservation?
    private var interruption: NSObjectProtocol?
    private var nextFrame: UInt32 = 0
    private var started = CACurrentMediaTime()
    private var stopped = false
    private var wakeup: DispatchSourceTimer?
    private var lastFrame = CACurrentMediaTime()
    /// The capture angle, which the preview follows.
    @MainActor private var angle: CGFloat?
    @MainActor weak var display: FrameLayerView? {
        didSet { if let angle { display?.rotate(angle) } }
    }

    init(session: MatrixCameraSession, update: @escaping @MainActor (Update) -> Void) {
        scan = session
        self.update = update
    }

    func start() {
        queue.async { [self] in
            // The system camera's device: a multi-camera one switches to the
            // ultra wide for focus when the card is held close.
            let types: [AVCaptureDevice.DeviceType] = [.builtInTripleCamera, .builtInDualWideCamera, .builtInWideAngleCamera]
            guard let device = types.lazy.compactMap({ AVCaptureDevice.default($0, for: .video, position: .back) }).first,
                  let input = try? AVCaptureDeviceInput(device: device),
                  capture.canAddInput(input), capture.canAddOutput(output)
            else {
                report(.failed(t("The camera is unavailable. Close other apps using it and try again.")))
                return
            }
            capture.beginConfiguration()
            capture.sessionPreset = .hd1920x1080
            capture.addInput(input)
            output.videoSettings = [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
            ]
            output.alwaysDiscardsLateVideoFrames = true
            output.setSampleBufferDelegate(self, queue: queue)
            capture.addOutput(output)
            capture.commitConfiguration()
            try? device.lockForConfiguration()
            // Focus on the centre, where the card is held, and refocus as it
            // moves; smooth focus is for filming and only slows the change.
            if device.isFocusPointOfInterestSupported { device.focusPointOfInterest = CGPoint(x: 0.5, y: 0.5) }
            if device.isFocusModeSupported(.continuousAutoFocus) { device.focusMode = .continuousAutoFocus }
            if device.isSmoothAutoFocusSupported { device.isSmoothAutoFocusEnabled = false }
            if device.isAutoFocusRangeRestrictionSupported { device.autoFocusRangeRestriction = .near }
            if device.isVirtualDevice {
                // 1× is the wide camera, as in the system camera; the device
                // leaves it for the ultra wide on its own.
                device.setPrimaryConstituentDeviceSwitchingBehavior(.auto, restrictedSwitchingBehaviorConditions: [])
                if let wide = device.virtualDeviceSwitchOverVideoZoomFactors.first {
                    device.videoZoomFactor = CGFloat(truncating: wide)
                }
            }
            device.activeVideoMaxFrameDuration = CMTime(value: 1, timescale: 30)
            device.unlockForConfiguration()
            // Frames arrive upright for the interface, so Core's normalized
            // outline maps straight onto the drawn picture.
            let coordinator = AVCaptureDevice.RotationCoordinator(device: device, previewLayer: nil)
            rotation = coordinator
            rotate(coordinator.videoRotationAngleForHorizonLevelCapture)
            rotationObservation = coordinator.observe(\.videoRotationAngleForHorizonLevelCapture) { [weak self] coordinator, _ in
                let angle = coordinator.videoRotationAngleForHorizonLevelCapture
                self?.queue.async { self?.rotate(angle) }
            }
            interruption = NotificationCenter.default.addObserver(
                forName: AVCaptureSession.wasInterruptedNotification, object: capture, queue: nil
            ) { [weak self] _ in
                self?.report(.failed(t("The camera disconnected. Open it again to continue.")))
                self?.stop()
            }
            started = CACurrentMediaTime()
            capture.startRunning()
            // Core's deadline advances even when no frame arrives.
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now() + 0.1, repeating: 0.1)
            timer.setEventHandler { [weak self] in self?.wake() }
            timer.resume()
            wakeup = timer
        }
    }

    /// Focuses and exposes at `point` (device coordinates) and keeps
    /// refocusing there while the card moves.
    func focus(at point: CGPoint) {
        queue.async { [self] in
            guard !stopped, let device = (capture.inputs.first as? AVCaptureDeviceInput)?.device,
                  (try? device.lockForConfiguration()) != nil else { return }
            if device.isFocusPointOfInterestSupported { device.focusPointOfInterest = point }
            if device.isFocusModeSupported(.continuousAutoFocus) { device.focusMode = .continuousAutoFocus }
            if device.isExposurePointOfInterestSupported {
                device.exposurePointOfInterest = point
                if device.isExposureModeSupported(.continuousAutoExposure) { device.exposureMode = .continuousAutoExposure }
            }
            device.unlockForConfiguration()
        }
    }

    func stop() {
        queue.async { [self] in
            guard !stopped else { return }
            stopped = true
            wakeup?.cancel()
            rotationObservation = nil
            if let interruption { NotificationCenter.default.removeObserver(interruption) }
            capture.stopRunning()
            scan.stop()
        }
    }

    private func rotate(_ angle: CGFloat) {
        Task { @MainActor [weak self] in
            self?.angle = angle
            self?.display?.rotate(angle)
        }
        guard let connection = output.connection(with: .video), connection.isVideoRotationAngleSupported(angle) else { return }
        connection.videoRotationAngle = angle
    }

    private func report(_ value: Update) {
        Task { @MainActor in update(value) }
    }

    private func wake() {
        guard !stopped, CACurrentMediaTime() - lastFrame >= 0.1 else { return }
        do { report(.state(try scan.poll())) } catch { fail(error) }
    }

    private func fail(_ error: Error) {
        guard !stopped else { return }
        report(.failed(AppModel.describe(error)))
        stop()
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput buffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard !stopped, let pixels = CMSampleBufferGetImageBuffer(buffer) else { return }
        lastFrame = CACurrentMediaTime()
        let frameId = nextFrame
        nextFrame &+= 1
        let frame: MatrixCameraFrame
        CVPixelBufferLockBaseAddress(pixels, .readOnly)
        do {
            defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
            guard CVPixelBufferGetPlaneCount(pixels) == 2,
                  let y = CVPixelBufferGetBaseAddressOfPlane(pixels, 0),
                  let uv = CVPixelBufferGetBaseAddressOfPlane(pixels, 1) else { return }
            let yStride = CVPixelBufferGetBytesPerRowOfPlane(pixels, 0)
            let uvStride = CVPixelBufferGetBytesPerRowOfPlane(pixels, 1)
            let chroma = Data(bytes: uv, count: uvStride * CVPixelBufferGetHeightOfPlane(pixels, 1))
            let matrix = CVBufferCopyAttachment(pixels, kCVImageBufferYCbCrMatrixKey, nil) as? String
            frame = MatrixCameraFrame(
                frameId: frameId,
                width: UInt32(CVPixelBufferGetWidth(pixels)),
                height: UInt32(CVPixelBufferGetHeight(pixels)),
                y: Data(bytes: y, count: yStride * CVPixelBufferGetHeightOfPlane(pixels, 0)),
                yRowStride: UInt32(yStride),
                // NV12 interleaves Cb and Cr: `v` is the same plane, one byte on.
                u: chroma,
                v: chroma.dropFirst(),
                uvRowStride: UInt32(uvStride),
                uvPixelStride: 2,
                bt709: matrix == (kCVImageBufferYCbCrMatrix_ITU_R_709_2 as String),
                fullRange: true,
                capturedMs: (CACurrentMediaTime() - started) * 1000
            )
        }
        let state: MatrixCameraState
        do { state = try scan.submit(frame: frame) } catch {
            fail(error)
            return
        }
        let size = CGSize(width: Int(frame.width), height: Int(frame.height))
        // Only an outline fitted on this frame; a stale one would point
        // where the card was.
        let outline = state.boundsFrameId == frameId ? state.bounds : []
        Task { @MainActor [weak self] in
            self?.display?.show(size: size, outline: outline)
            self?.update(.state(state))
        }
        if state.phase == .complete { stop() }
    }
}

