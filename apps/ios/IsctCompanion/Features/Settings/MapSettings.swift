import SwiftUI

/// Settings → Map: which maps app the campus map's "Open in Maps" links
/// open in. Core builds the link from this device setting.
struct MapSection: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Section(t("settings.main.map")) {
            Picker(selection: Binding(
                get: { model.records.settings.text("mapsApp") ?? "apple" },
                set: { value in Task { await model.updateSettings { $0["mapsApp"] = value } } }
            )) {
                Text(t("settings.main.appleMaps")).tag("apple")
                Text(t("settings.main.googleMaps")).tag("google")
            } label: {
                Label { Text(t("settings.main.openPlacesIn")) } icon: { SettingsIcon(symbol: "map", tint: Theme.ok) }
            }
        }
    }
}
