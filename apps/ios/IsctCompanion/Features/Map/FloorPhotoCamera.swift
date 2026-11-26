import SwiftUI
import UIKit

extension CampusMapScreen {
    func floorPhotoCamera(_ content: some View) -> some View {
        content
            .fullScreenCover(isPresented: Binding(get: { cameraFor != nil }, set: { if !$0 { cameraFor = nil } })) {
                SystemCamera { image in
                    let building = cameraFor
                    cameraFor = nil
                    if let image, let building { Task { await sendPhoto(image, of: building) } }
                }
                .ignoresSafeArea()
            }
            .safeAreaInset(edge: .bottom) {
                if let photoFailure {
                    ProblemBanner(text: "\(t("campusMap.photos.failed"))\n\(photoFailure)", dismiss: {
                        self.photoFailure = nil
                    })
                    .padding(.horizontal, Theme.gutter)
                }
            }
    }

    /// Sends a photo in the background; the card's button tells how it went.
    private func sendPhoto(_ image: UIImage, of building: String) async {
        guard let jpeg = await Task.detached(priority: .userInitiated, operation: { floorPhotoJPEG(image) }).value
        else { return }
        photoUploads[building, default: FloorPhotoUploads()].sending += 1
        let refused = await model.campusMapUploadPhoto(buildingId: building, jpeg: jpeg)
        photoUploads[building, default: FloorPhotoUploads()].sending -= 1
        if let refused {
            photoFailure = refused.message
        } else {
            photoUploads[building, default: FloorPhotoUploads()].sent += 1
            mapPicks += 1
        }
    }
}

/// A building Core asks floor photos of.
/// A building's photo uploads during this map visit.
struct FloorPhotoUploads: Equatable {
    var sending = 0
    var sent = 0
}

/// Upright pixels at most 2048 px on the long side, as JPEG: drawing the
/// photo anew keeps none of the camera's metadata, so no EXIF and no
/// location leave the device. Core signs it for the maintainers' inbox.
private func floorPhotoJPEG(_ image: UIImage) -> Data? {
    let longest = max(image.size.width, image.size.height)
    guard longest > 0 else { return nil }
    let scale = min(1, 2048 / longest)
    let size = CGSize(width: (image.size.width * scale).rounded(), height: (image.size.height * scale).rounded())
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    format.opaque = true
    let drawn = UIGraphicsImageRenderer(size: size, format: format).image { _ in
        image.draw(in: CGRect(origin: .zero, size: size))
    }
    return drawn.jpegData(compressionQuality: 0.8)
}
