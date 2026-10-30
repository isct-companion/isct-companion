import IsctCore
import SwiftUI

/// データ更新: whether Core's automatic school reads may use cellular data
/// (`settings.subscriptionRefresh.allowCellular`), Core's sync sources and
/// runs, and a manual refresh that asks Core's planner for everything.
struct DataUpdatesScreen: View {
    @Environment(AppModel.self) private var model

    private var refresh: [String: Any] {
        model.records.settings["subscriptionRefresh"] as? [String: Any] ?? [:]
    }

    var body: some View {
        Form {
            Group {
                Section {
                    Toggle(t("settings.main.updateOnCellularData"), isOn: Binding(
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
                    Text(t("settings.main.whenOffAutomaticUpdatesWait"))
                }
                SyncSections()
                Section {
                    Button {
                        model.refreshFromSchool()
                    } label: {
                        HStack {
                            Text(t("common.refreshNow"))
                            Spacer()
                            if let indicator = model.syncSnapshot?.indicator {
                                SyncIndicatorView(indicator: indicator)
                            }
                        }
                    }
                }
            }
            .themedRows()
        }
        .themedList()
        .navigationTitle(t("settings.main.dataUpdates"))
        .navigationBarTitleDisplayMode(.inline)
    }
}
