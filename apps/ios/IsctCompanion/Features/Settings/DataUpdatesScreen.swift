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
                Toggle("モバイル通信でも更新", isOn: Binding(
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
                Text("オフのときは Wi-Fi に接続するまで自動更新を待ちます。")
            }
            if let sources = model.schoolStatus?.sources, !sources.isEmpty {
                Section("最終更新") {
                    ForEach(sources, id: \.site) { source in
                        LabeledContent(WebRouter.name(source.site)) {
                            Text(source.lastUpdated.map { Format.due($0) } ?? "未取得")
                        }
                    }
                }
            }
            Section {
                Button {
                    model.refreshFromSchool()
                } label: {
                    HStack {
                        Text("今すぐ更新")
                        Spacer()
                        if let status = model.schoolStatus, let progress = status.headline.ringProgress(status) {
                            SyncRing(progress: progress, emphasized: status.headline.isBusy)
                        }
                    }
                }
                .disabled(model.schoolStatus?.headline.isBusy == true)
            }
        }
        .navigationTitle("データ更新")
        .navigationBarTitleDisplayMode(.inline)
    }
}
