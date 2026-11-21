import IsctCore
import SwiftUI

func OpenReleaseButton(release: AppRelease, openURL: OpenURLAction) -> Button<Text>? {
    guard let url = URL(string: release.url) else { return nil }
    return Button(t("appUpdate.openAppStore")) { openURL(url) }
}
