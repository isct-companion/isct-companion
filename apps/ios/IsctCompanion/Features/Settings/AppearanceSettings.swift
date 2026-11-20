import SwiftUI

/// Settings → the background theme: five swatches, applied at once and kept
/// in the synced settings (slotledger `approach/neutral-themes`).
struct ThemeSection: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var scheme

    private static let names = ["warm": "settings.main.warm", "cool": "settings.main.cool", "paper": "settings.main.paper", "mist": "settings.main.mist", "moss": "settings.main.moss"]

    var body: some View {
        let chosen = model.records.settings.text("theme") ?? "warm"
        Section {
            HStack(spacing: 0) {
                Label { Text(t("settings.main.theme")) } icon: { SettingsIcon(symbol: "paintpalette", tint: Theme.accent) }
                Spacer(minLength: Theme.space6)
                HStack(spacing: Theme.space5) {
                    ForEach(Theme.themes, id: \.self) { id in swatch(id, selected: id == chosen) }
                }
            }
            .sensoryFeedback(.selection, trigger: chosen)
        }
    }

    private func swatch(_ id: String, selected: Bool) -> some View {
        let set = IsctThemes.all[id].map { scheme == .dark ? $0.dark : $0.light }
        return Button {
            Task { await model.updateSettings { $0["theme"] = id } }
        } label: {
            Circle()
                .fill(set?.bg ?? Theme.bg)
                .overlay { Circle().fill(set?.surface ?? Theme.surface).padding(Theme.space4) }
                .overlay { Circle().strokeBorder(selected ? Theme.accent : Theme.line, lineWidth: selected ? 2 : 1) }
                .frame(width: 28, height: 28)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(t(Self.names[id] ?? id))
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Settings → Map: which maps app the campus map's "Open in Maps" links

/// The desktop's language rows: the UI language and the language of school
/// text. Both are workspace settings, so they follow the person to desktop.
struct LanguageSection: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Section {
            Picker(selection: binding("locale", default: "en")) {
                Text(verbatim: "English").tag("en")
                Text(verbatim: "日本語").tag("ja")
                Text(verbatim: "中文").tag("zh")
            } label: {
                Label { Text(t("common.language")) } icon: { SettingsIcon(symbol: "character.bubble", tint: Theme.accent) }
            }
            Picker(selection: binding("contentLanguage", default: "auto")) {
                Text(t("common.automatic")).tag("auto")
                Text(verbatim: "English").tag("en")
                Text(verbatim: "日本語").tag("ja")
            } label: {
                Label { Text(t("settings.main.schoolContentLanguage")) } icon: { SettingsIcon(symbol: "text.book.closed", tint: Theme.accent) }
            }
            Toggle(t("settings.appearance.showOneLanguageInBilingual"), isOn: Binding(
                get: { model.records.settings["lmsBilingualFilter"] as? Bool == true },
                set: { value in Task { await model.updateSettings { $0["lmsBilingualFilter"] = value } } }
            ))
        } footer: {
            Text(t("settings.main.courseNamesRoomsAndSyllabus"))
        }
    }

    private func binding(_ key: String, default fallback: String) -> Binding<String> {
        Binding(
            get: { model.records.settings.text(key) ?? fallback },
            set: { value in Task { await model.updateSettings { $0[key] = value } } }
        )
    }
}
