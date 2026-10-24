import AVFoundation
import IsctCore
import SwiftUI
import UIKit

/// Live Matrix scanning: AVFoundation hands each NV12 frame to Core's camera
/// session (the same driver as the desktop camera and photo import) and draws
/// the frame it analysed with the outline Core fitted on that frame, so the
/// box never lags the picture. The grid fills in as Core confirms cells.
struct MatrixCameraScreen: View {
    let session: MatrixCameraSession
    /// The final cells, or nil when the scan was cancelled or failed.
    let done: ([MatrixCell]?) -> Void
    @State private var camera: MatrixCameraController?
    @State private var state: MatrixCameraState?
    @State private var error: String?
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        VStack(spacing: 12) {
            ZStack {
                Color.black
                if let camera { FrameView(controller: camera) }
            }
            .clipShape(RoundedRectangle(cornerRadius: 12))
            LiveGrid(cells: state?.cells ?? [])
            Text(error ?? status)
                .font(.footnote)
                .foregroundStyle(error == nil ? Theme.muted : Theme.danger)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
            Button(t("Cancel"), role: .cancel) { finish(nil) }
                .buttonStyle(.bordered)
        }
        .padding()
        .background(Theme.bg)
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
                            .foregroundStyle(cell?.stable == true ? Theme.ink : Theme.muted)
                            .frame(maxWidth: .infinity, minHeight: 22)
                            .background(cell?.stable == true ? Theme.okSoft : Theme.surface2,
                                        in: RoundedRectangle(cornerRadius: 4))
                    }
                }
            }
        }
        .privacySensitive()
    }
}

/// Shows the analysed frames (`AVSampleBufferDisplayLayer`) and the outline.
private struct FrameView: UIViewRepresentable {
    let controller: MatrixCameraController

    func makeUIView(context: Context) -> FrameLayerView {
        let view = FrameLayerView()
        controller.display = view
        return view
    }

    func updateUIView(_ view: FrameLayerView, context: Context) {}
}

final class FrameLayerView: UIView {
    private let video = AVSampleBufferDisplayLayer()
    private let outline = CAShapeLayer()
    private var imageSize = CGSize(width: 1, height: 1)
    private var bounds4: [MatrixPoint] = []

    override init(frame: CGRect) {
        super.init(frame: frame)
        video.videoGravity = .resizeAspect
        outline.fillColor = nil
        outline.strokeColor = UIColor(red: 1, green: 0.27, blue: 0.27, alpha: 1).cgColor
        outline.lineWidth = 3
        layer.addSublayer(video)
        layer.addSublayer(outline)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func layoutSubviews() {
        super.layoutSubviews()
        video.frame = bounds
        outline.frame = bounds
        drawOutline()
    }

    /// The frame Core just analysed, and the outline fitted on that frame.
    func show(_ buffer: CMSampleBuffer, size: CGSize, outline points: [MatrixPoint]) {
        imageSize = size
        bounds4 = points
        video.sampleBufferRenderer.enqueue(buffer)
        drawOutline()
    }

    private func drawOutline() {
        guard bounds4.count == 4 else {
            outline.path = nil
            return
        }
        let rect = AVMakeRect(aspectRatio: imageSize, insideRect: bounds)
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
    private let capture = AVCaptureSession()
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
    @MainActor weak var display: FrameLayerView?

    init(session: MatrixCameraSession, update: @escaping @MainActor (Update) -> Void) {
        scan = session
        self.update = update
    }

    func start() {
        queue.async { [self] in
            guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back),
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
            if device.isFocusModeSupported(.continuousAutoFocus) { device.focusMode = .continuousAutoFocus }
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
        // Show this frame only with an outline from this same frame.
        let outline = state.boundsFrameId == frameId ? state.bounds : []
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(buffer, createIfNecessary: true),
           CFArrayGetCount(attachments) > 0 {
            let first = unsafeBitCast(CFArrayGetValueAtIndex(attachments, 0), to: CFMutableDictionary.self)
            CFDictionarySetValue(first,
                                 Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
                                 Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
        }
        let display = UncheckedBuffer(buffer)
        Task { @MainActor [weak self] in
            self?.display?.show(display.buffer, size: size, outline: outline)
            self?.update(.state(state))
        }
        if state.phase == .complete { stop() }
    }
}

/// `CMSampleBuffer` is immutable once captured; it only crosses to the main
/// actor to be drawn.
private struct UncheckedBuffer: @unchecked Sendable {
    let buffer: CMSampleBuffer
    init(_ buffer: CMSampleBuffer) { self.buffer = buffer }
}
