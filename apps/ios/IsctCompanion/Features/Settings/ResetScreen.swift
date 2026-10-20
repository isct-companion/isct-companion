import IsctCore
import SwiftUI

/// Core's two reset scopes, as the pages that confirm them.
enum ResetKind: Hashable {
    case schoolCaches, everything
}

/// Settings → Reset, the desktop's `ResetSettings` as a page: clear the school
/// caches and read them again, or return this device to its first-launch
/// state. It lists what goes and what stays, with Core's estimate of the
/// space freed; a full reset needs the acknowledgement and one more
/// confirmation. Core decides what each scope removes.
struct ResetScreen: View {
    let kind: ResetKind
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var signOutWeb = false
    @State private var deleteDownloads = false
    @State private var acknowledged = false
    @State private var restore = true
    @State private var estimate: Estimate = .estimating
    @State private var confirming = false
    @State private var resetting = false
    @State private var notice: Notice?

    private enum Estimate {
        case estimating, unavailable, bytes(UInt64)
    }

    private struct Notice: Identifiable {
        let text: String
        let leave: Bool
        var id: String { text }
    }

    /// What every reset removes; the item keys live under `settings.reset.item`.
    private static let schoolCaches = ["schoolData", "catalog", "references", "syncRecords", "lmsToken"]

    private var everything: Bool { kind == .everything }
    private var cloudOn: Bool { model.cloud?.providers.contains { $0.enabled } == true }

    private var deletes: [String] {
        everything
            ? Self.schoolCaches + ["webSession", "userData", "settings", "logins"]
            : Self.schoolCaches + (signOutWeb ? ["webSession"] : []) + (deleteDownloads ? ["downloads"] : [])
    }

    private var keeps: [String] {
        everything
            ? ["downloads", "cloud", "logs"]
            : ["userData", "settings", "logins"] + (signOutWeb ? [] : ["webSession"]) + (deleteDownloads ? [] : ["downloads"])
    }

    private var request: ResetRequest {
        everything ? .everything : .schoolCaches(signOutWeb: signOutWeb, deleteDownloads: deleteDownloads)
    }

    private var confirmLabel: String {
        t(everything ? "settings.reset.full.confirm" : "settings.reset.caches.confirm")
    }

    var body: some View {
        Form {
            if !everything {
                Section {
                    option(t("settings.reset.caches.signOut"), t("settings.reset.caches.signOutHint"), $signOutWeb)
                    option(t("settings.reset.caches.downloads"), t("settings.reset.caches.downloadsHint"), $deleteDownloads)
                } footer: {
                    Text(t("settings.reset.caches.hint"))
                }
            }
            items("settings.reset.deletes", deletes, icon: "trash")
            items("settings.reset.keeps", keeps, icon: "checkmark")
            Section {
                LabeledContent(t("settings.reset.freed"), value: freed)
            } footer: {
                if everything { Text(t("settings.reset.full.hint")) }
            }
            if everything, cloudOn {
                Section(t("settings.reset.full.cloudQuestion")) {
                    Picker(t("settings.reset.full.cloudQuestion"), selection: $restore) {
                        choice(t("settings.reset.full.restore"), t("settings.reset.full.restoreHint")).tag(true)
                        choice(t("settings.reset.full.newDevice"), t("settings.reset.full.newDeviceHint")).tag(false)
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()
                }
            }
            if everything {
                Section {
                    Toggle(t("settings.reset.full.acknowledge"), isOn: $acknowledged)
                }
            }
            Section {
                Button(role: .destructive) {
                    if everything { confirming = true } else { Task { await reset() } }
                } label: {
                    if resetting { Text(t("settings.reset.resetting")) } else { Text(confirmLabel) }
                }
                .disabled(resetting || (everything && !acknowledged))
            }
        }
        .navigationTitle(t(everything ? "settings.reset.full.title" : "settings.reset.caches.title"))
        .navigationBarTitleDisplayMode(.inline)
        .disabled(resetting)
        .interactiveDismissDisabled(resetting)
        .navigationBarBackButtonHidden(resetting)
        // Signing out frees no space, so only the downloads option re-estimates.
        .task(id: deleteDownloads) {
            estimate = .estimating
            let preview = await model.resetPreview(
                everything ? .everything : .schoolCaches(signOutWeb: false, deleteDownloads: deleteDownloads))
            guard !Task.isCancelled else { return }
            estimate = preview.map { .bytes($0.freedBytes) } ?? .unavailable
        }
        .confirmationDialog(
            t("settings.reset.full.title"), isPresented: $confirming, titleVisibility: .visible
        ) {
            Button(confirmLabel, role: .destructive) { Task { await reset() } }
            Button(t("settings.reset.cancel"), role: .cancel) {}
        } message: {
            Text(t("settings.reset.full.acknowledge"))
        }
        .alert(item: $notice) { notice in
            Alert(title: Text(notice.text), dismissButton: .default(Text("OK")) { if notice.leave { dismiss() } })
        }
    }

    private var freed: String {
        switch estimate {
        case .estimating: t("settings.reset.estimating")
        case .unavailable: "–"
        case let .bytes(bytes): Int64(clamping: bytes).formatted(.byteCount(style: .file).locale(Localizer.shared.locale))
        }
    }

    private func reset() async {
        let restoring = everything && cloudOn && restore
        resetting = true
        defer { resetting = false }
        do {
            try await model.reset(request)
        } catch {
            notice = Notice(text: Self.failure(error), leave: false)
            return
        }
        if restoring {
            do {
                try await model.restoreFromCloud()
            } catch {
                notice = Notice(text: t("settings.reset.full.restoreFailed"), leave: true)
                return
            }
        }
        notice = Notice(text: t(everything ? "settings.reset.full.done" : "settings.reset.caches.done"), leave: true)
    }

    private static func failure(_ error: Error) -> String {
        let text = AppModel.describe(error)
        return text.isEmpty ? t("settings.reset.failed") : text
    }

    private func option(_ title: String, _ hint: String, _ isOn: Binding<Bool>) -> some View {
        Toggle(isOn: isOn) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(hint).font(.footnote).foregroundStyle(Theme.muted)
            }
        }
    }

    private func choice(_ title: String, _ hint: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
            Text(hint).font(.footnote).foregroundStyle(Theme.muted)
        }
    }

    private func items(_ title: String, _ keys: [String], icon: String) -> some View {
        Section(t(title)) {
            ForEach(keys, id: \.self) { key in
                Label(t("settings.reset.item.\(key)"), systemImage: icon)
            }
        }
    }
}
