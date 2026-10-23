import IsctCore
import SwiftUI
import UniformTypeIdentifiers

/// バックアップ: the desktop's Backup settings on the phone. iCloud carries the
/// same record the Mac app writes and Google Drive the one every platform
/// writes, so any device sharing a provider loads the others' data; a backup
/// file moves data between any two devices. Core owns every rule
/// (what uploads, how copies merge, what a restore keeps); this renders its
/// journal and forwards the person's choices.
struct BackupScreen: View {
    @Environment(AppModel.self) private var model
    @State private var busy = false
    @State private var confirm: Confirmation?
    @State private var exporting: ExportFile?
    @State private var importing = false
    @State private var notice: String?

    private var state: SyncState? { model.cloud }
    private var providers: [ProviderState] { state?.providers ?? [] }
    private var pending: Bool {
        state?.tasks.contains { $0.deliveries.contains { $0.status != .completed } } ?? false
    }
    private var locked: Bool { busy || state?.syncing == true }

    var body: some View {
        Form {
            syncSection
            if let versions = state?.versions, !versions.isEmpty {
                Section {
                    ForEach(versions.sorted { $0.savedAt > $1.savedAt }, id: \.id) { version in
                        restoreRow(Format.due(version.savedAt), detail: version.device, id: version.id, savedAt: version.savedAt)
                    }
                } header: {
                    Text(t("Earlier versions"))
                } footer: {
                    Text(t("Backups older than 30 days are removed only when at least four newer backups exist."))
                }
            }
            if let points = state?.recoveryPoints, !points.isEmpty {
                Section {
                    ForEach(points.sorted { $0.savedAt > $1.savedAt }, id: \.id) { point in
                        restoreRow(Format.due(point.savedAt), detail: nil, id: point.id, savedAt: point.savedAt)
                    }
                } header: {
                    Text(t("Local recovery files"))
                } footer: {
                    Text(t("Saved before cloud changes or a restore replace local content. Backups older than 30 days are removed only when at least four newer backups exist."))
                }
            }
            Section {
                Button(t("Export…")) { Task { await export() } }
                Button(t("Import…")) { importing = true }
            } header: {
                Text(t("Backup file"))
            } footer: {
                Text(t("School logins sync with iCloud Keychain."))
            }
        }
        .navigationTitle(t("Backup"))
        .navigationBarTitleDisplayMode(.inline)
        .disabled(busy)
        .task { await model.refreshCloud() }
        .confirmationDialog(
            confirm?.message ?? "", isPresented: Binding(get: { confirm != nil }, set: { if !$0 { confirm = nil } }),
            titleVisibility: .visible, presenting: confirm
        ) { confirmation in
            Button(t("Restore"), role: .destructive) { Task { await confirmation.action() } }
        }
        .fileExporter(
            isPresented: Binding(get: { exporting != nil }, set: { if !$0 { exporting = nil } }),
            document: exporting, contentType: .json, defaultFilename: "IsctCompanion.json"
        ) { _ in exporting = nil }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.json]) { result in
            guard case let .success(url) = result else { return }
            confirm = Confirmation(
                message: t("Restore this file on all connected devices? A recovery copy of current data will be kept.")
            ) { await importFile(url) }
        }
        .alert(notice ?? "", isPresented: Binding(get: { notice != nil }, set: { if !$0 { notice = nil } })) {
            Button("OK", role: .cancel) {}
        }
    }

    @ViewBuilder private var syncSection: some View {
        Section {
            ForEach(providers, id: \.provider) { provider in
                Toggle(isOn: Binding(
                    get: { provider.enabled },
                    set: { on in run { await model.setCloudEnabled(provider.provider, on) } })
                ) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(Self.name(provider.provider))
                        if let detail = detail(provider) {
                            Text(detail).font(.subheadline)
                                .foregroundStyle(provider.error == nil ? Theme.muted : Theme.danger)
                        }
                    }
                }
                .disabled(locked)
            }
            if providers.contains(where: \.enabled) {
                Toggle(isOn: Binding(
                    get: { state?.automatic ?? false },
                    set: { on in run { await model.setCloudAutomatic(on) } })
                ) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(t("Sync automatically"))
                        Text(state?.automatic == true
                            ? t("Pulls other devices' changes when opened and every hour. Changed content is uploaded at most once per hour.")
                            : t("Upload and pull by hand"))
                            .font(.subheadline).foregroundStyle(Theme.muted)
                    }
                }
                .disabled(locked)
                HStack {
                    Button(t("Upload")) { run { await model.syncCloud(.upload) } }
                        .buttonStyle(.borderless)
                    Spacer()
                    if state?.syncing == true {
                        ProgressView()
                        Text(t("Syncing…")).foregroundStyle(Theme.muted)
                        Spacer()
                    }
                    Button(t("Pull")) { run { await model.syncCloud(.pull) } }
                        .buttonStyle(.borderless)
                }
                .disabled(locked)
            }
        } header: {
            Text(t("Sync"))
        }
    }

    private static func name(_ provider: CloudProvider) -> String {
        switch provider {
        case .icloud: "iCloud"
        case .googleDrive: "Google Drive"
        case .onedrive: "OneDrive"
        }
    }

    private func detail(_ provider: ProviderState) -> String? {
        // The provider's own reason (signed out, declined sign-in, offline):
        // without it every failure looks the same. A declined sign-in leaves
        // the switch off with its reason.
        if let error = provider.error {
            return t("Could not update this cloud copy. Your data is saved on this device.") + "\n" + error
        }
        guard provider.enabled else { return nil }
        return provider.lastSyncedAt.map { t("Synced {{date}}", ["date": Format.due($0)]) }
            ?? t("Waiting for the first sync")
    }

    private func restoreRow(_ title: String, detail: String?, id: String, savedAt: String) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                if let detail { Text(detail).font(.subheadline).foregroundStyle(Theme.muted) }
            }
            Spacer()
            Button(t("Restore")) {
                confirm = Confirmation(message: t(
                    "Restore the version from {{date}} on all connected devices? Unsynced work on older devices will be kept in recovery files.",
                    ["date": Format.due(savedAt)])
                ) {
                    await model.restoreCloud(id)
                    notice = t("Restored the earlier version.")
                }
            }
            .buttonStyle(.borderless)
            // Core refuses a restore while a transfer is unfinished.
            .disabled(locked || pending)
        }
    }

    private func run(_ work: @escaping () async -> Void) {
        Task {
            busy = true
            await work()
            busy = false
        }
    }

    private func export() async {
        guard let text = await model.exportBackup() else { return }
        exporting = ExportFile(data: Data(text.utf8))
    }

    private func importFile(_ url: URL) async {
        busy = true
        defer { busy = false }
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            try await model.importBackup(try Data(contentsOf: url))
            notice = t("Backup restored.")
        } catch {
            notice = t("Import failed: {{reason}}", ["reason": AppModel.describe(error)])
        }
    }
}

private struct Confirmation {
    let message: String
    let action: () async -> Void
}

/// An exported file (workspace backup, application logs), handed to the
/// system's save sheet.
struct ExportFile: FileDocument {
    static let readableContentTypes: [UTType] = [.json, .plainText]
    let data: Data

    init(data: Data) { self.data = data }

    init(configuration: ReadConfiguration) throws {
        data = configuration.file.regularFileContents ?? Data()
    }

    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: data)
    }
}
