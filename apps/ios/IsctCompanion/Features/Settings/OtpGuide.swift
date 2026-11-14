import SwiftUI
import UIKit

/// Step-by-step OTP setup guides (slotledger `approach/otp-setup-guide`): the
/// same screenshots and step list as the desktop and Android, bundled from
/// `resources/otp-guide`. Each step is a screenshot, where to tap on it (as a
/// fraction of its width and height) or `screenshot` when the person should
/// capture that screen, and a caption key.
enum OtpGuide: String, CaseIterable, Identifiable {
    case google, titechApp

    var id: String { rawValue }

    struct Step: Decodable {
        struct Point: Decodable { let x: Double; let y: Double }
        let image: String
        let tap: Point?
        let action: String?
        let caption: String
    }

    private static let all: [String: [Step]] = {
        guard let url = Bundle.main.url(forResource: "otp-guide", withExtension: "json"),
            let data = try? Data(contentsOf: url)
        else { return [:] }
        return (try? JSONDecoder().decode([String: [Step]].self, from: data)) ?? [:]
    }()

    var steps: [Step] { Self.all[rawValue] ?? [] }

    var title: String {
        switch self {
        case .google: t("otpGuide.google.title")
        case .titechApp: t("otpGuide.titechApp.title")
        }
    }
}

/// "Show steps" for an import guide.
struct OtpGuideButton: View {
    let guide: OtpGuide
    @State private var shown: OtpGuide?

    var body: some View {
        Button(t("otpGuide.showSteps"), systemImage: "hand.tap") { shown = guide }
            .sheet(item: $shown) { OtpGuideSheet(guide: $0) }
    }
}

/// One page per step, swiped like the system's own setup guides.
private struct OtpGuideSheet: View {
    let guide: OtpGuide
    @State private var page = 0
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let steps = guide.steps
        NavigationStack {
            TabView(selection: $page) {
                ForEach(Array(steps.enumerated()), id: \.offset) { index, step in
                    VStack(spacing: 16) {
                        Screenshot(step: step)
                        Text(t(step.caption))
                            .font(.callout)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.horizontal, Theme.gutter)
                    .padding(.bottom, 48)
                    .tag(index)
                }
            }
            .tabViewStyle(.page(indexDisplayMode: .always))
            .indexViewStyle(.page(backgroundDisplayMode: .always))
            .background(Theme.bg)
            .navigationTitle(guide.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("common.done")) { dismiss() }
                }
            }
        }
    }
}

/// The screenshot sized to fit, with the tap marker at the step's point.
private struct Screenshot: View {
    let step: OtpGuide.Step

    var body: some View {
        if let path = Bundle.main.path(forResource: step.image, ofType: nil),
            let image = UIImage(contentsOfFile: path)
        {
            Image(uiImage: image)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .overlay {
                    if let tap = step.tap {
                        GeometryReader { frame in
                            TapMarker()
                                .position(x: tap.x * frame.size.width, y: tap.y * frame.size.height)
                        }
                    } else if step.action == "screenshot" {
                        CaptureMarker(image: image)
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: Theme.radiusPanel))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusPanel).stroke(Theme.line))
                .accessibilityHidden(true)
        }
    }
}

/// A ring that keeps pulsing out of a dot, like a touch indicator in a screen
/// recording; with Reduce Motion it holds still. White edges keep it visible
/// on light and dark screenshots.
private struct TapMarker: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pulsing = false

    var body: some View {
        ZStack {
            Circle()
                .strokeBorder(Theme.accent, lineWidth: 3)
                .background(Circle().strokeBorder(.white.opacity(0.85), lineWidth: 5))
                .scaleEffect(reduceMotion ? 1 : (pulsing ? 1.6 : 0.5))
                .opacity(reduceMotion ? 1 : (pulsing ? 0 : 1))
            Circle()
                .fill(Theme.accent.opacity(0.7))
                .overlay(Circle().strokeBorder(.white.opacity(0.85), lineWidth: 2))
                .frame(width: 20, height: 20)
        }
        .frame(width: 40, height: 40)
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeOut(duration: 1.6).repeatForever(autoreverses: false)) { pulsing = true }
        }
    }
}

/// "Take a screenshot": the screen flashes, then shrinks into a thumbnail in
/// the corner, the way the system shows a capture; with Reduce Motion only
/// the thumbnail shows.
private struct CaptureMarker: View {
    let image: UIImage
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private enum Phase: CaseIterable { case idle, flash, shrunk, held }

    var body: some View {
        if reduceMotion {
            frame(flash: 0, scale: 1, shown: true)
        } else {
            PhaseAnimator(Phase.allCases) { phase in
                frame(
                    flash: phase == .flash ? 0.85 : 0,
                    scale: phase == .flash ? 3.2 : 1,
                    shown: phase != .idle)
            } animation: { phase in
                switch phase {
                case .idle: .easeOut(duration: 0.4)
                case .flash: .linear(duration: 0.12)
                case .shrunk: .spring(duration: 0.5, bounce: 0)
                case .held: .linear(duration: 1.4)
                }
            }
        }
    }

    private func frame(flash: Double, scale: CGFloat, shown: Bool) -> some View {
        GeometryReader { frame in
            ZStack(alignment: .bottomLeading) {
                Color.white.opacity(flash)
                Image(uiImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(width: frame.size.width * 0.3)
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.white, lineWidth: 3))
                    .shadow(color: .black.opacity(0.3), radius: 7, y: 3)
                    .scaleEffect(scale, anchor: .bottomLeading)
                    .opacity(shown ? 1 : 0)
                    .padding(.leading, frame.size.width * 0.05)
                    .padding(.bottom, frame.size.height * 0.04)
            }
        }
        .allowsHitTesting(false)
    }
}
