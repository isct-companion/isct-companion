import SwiftUI
import UniformTypeIdentifiers

/// What the campus printer takes and the two ways in, opened from 今日's
/// print button, as on Android: share a file to "Print via campus printer"
/// from another app, or pick one here and go straight to the print screen.
/// The picker offers only what the print screen prints, PDFs and pictures.
struct PrintGuideRoute: Hashable {}

struct PrintGuideScreen: View {
    @Environment(\.openCampusPrint) private var openCampusPrint
    @State private var picking = false
    @State private var unreadable = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Theme.space4) {
                Text(t("campusPrint.guide.intro"))
                    .font(.subheadline)
                    .foregroundStyle(Theme.muted)
                way(systemImage: "square.and.arrow.up", title: t("campusPrint.guide.shareTitle")) {
                    Text(t("campusPrint.guide.shareIos"))
                }
                way(systemImage: "folder", title: t("campusPrint.guide.pickTitle")) {
                    Text(t("campusPrint.guide.pickBody"))
                    Button { picking = true } label: {
                        Label(t("campusPrint.guide.pickButton"), systemImage: "folder")
                            .font(.headline)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 6)
                    }
                    .buttonStyle(.borderedProminent)
                    .padding(.top, Theme.space2)
                }
                Label {
                    Text(t("campusPrint.guide.formats"))
                } icon: {
                    Image(systemName: "info.circle.fill").foregroundStyle(Theme.warn)
                }
                .font(.subheadline)
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Theme.warnSoft, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
            }
            .padding(.horizontal, Theme.gutter)
            .padding(.bottom, Theme.space10)
        }
        .background(Theme.bg)
        .navigationTitle(t("campusPrint.guide.title"))
        .navigationBarTitleDisplayMode(.inline)
        .fileImporter(isPresented: $picking, allowedContentTypes: [.pdf, .image]) { result in
            guard case let .success(url) = result else { return }
            if let copy = Self.copy(url) {
                openCampusPrint(copy, url.lastPathComponent)
            } else {
                unreadable = true
            }
        }
        .alert(t("campusPrint.window.couldNotReadTheFile"), isPresented: $unreadable) {
            Button(t("common.close"), role: .cancel) {}
        }
    }

    private func way(systemImage: String, title: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(.title3)
                    .foregroundStyle(Theme.accent)
                    .frame(width: 40, height: 40)
                    .background(Theme.accentSoft, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                Text(title).font(.headline).foregroundStyle(Theme.ink)
            }
            content()
                .foregroundStyle(Theme.ink)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
    }

    /// The picked file, copied out of its security scope into the app's
    /// temporary folder, so the print screen can read it after the picker
    /// is gone.
    private static func copy(_ url: URL) -> URL? {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("print-pick", isDirectory: true)
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let copy = folder.appendingPathComponent(url.lastPathComponent)
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try FileManager.default.copyItem(at: url, to: copy)
            return copy
        } catch {
            return nil
        }
    }
}
