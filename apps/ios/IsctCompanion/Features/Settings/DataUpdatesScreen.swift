import IsctCore
import SwiftUI

/// データ更新: whether Core's automatic school reads may use cellular data
/// (`settings.subscriptionRefresh.allowCellular`, read by `school_refresh.rs`),
/// when each school site was last read, and a manual refresh.
struct DataUpdatesScreen: View {
    @Environment(AppModel.self) private var model

    private var refresh: [String: Any] {
        model.records.settings["subscriptionRefresh"] as? [String: Any] ?? [:]
    }

    var body: some View {
        Form {
            Section {
                Toggle(t("Update on cellular data"), isOn: Binding(
                    get: { refresh["allowCellular"] as? Bool ?? false },
                    set: { on in
                        Task {
                            await model.updateSettings { settings in
                                var refresh = settings["subscriptionRefresh"] as? [String: Any] ?? [:]
                                refresh["allowCellular"] = on
                                settings["subscriptionRefresh"] = refresh
                            }
                        }
                    }))
            } footer: {
                Text(t("When off, automatic updates wait until you are on Wi-Fi."))
            }
            if let sources = model.schoolStatus?.sources, !sources.isEmpty {
                Section(t("Last updated")) {
                    ForEach(sources, id: \.site) { source in
                        LabeledContent(WebRouter.name(source.site)) {
                            Text(source.lastUpdated.map { Format.due($0) } ?? t("Not fetched yet"))
                        }
                    }
                }
            }
            Section {
                Button {
                    model.refreshFromSchool()
                } label: {
                    HStack {
                        Text(t("Refresh now"))
                        Spacer()
                        if let status = model.schoolStatus, let progress = status.headline.ringProgress(status) {
                            SyncRing(progress: progress, emphasized: status.headline.isBusy)
                        }
                    }
                }
                .disabled(model.schoolStatus?.headline.isBusy == true)
            }
        }
        .navigationTitle(t("Data updates"))
        .navigationBarTitleDisplayMode(.inline)
    }
}
