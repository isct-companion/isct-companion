package app.isctcompanion.schoolweb

import org.junit.Assert.assertEquals
import org.junit.Test

class SchoolFrameTest {
    @Test fun convertsCssPixelsAndClipsBelowTheToolbar() {
        assertEquals(SchoolFrame(0, 240, 1080, 2088),
            SchoolFrame.calculate(0.0, 56.0, 360.0, 700.0, 3.0, 0, 72, 0, 72, 1080, 2328))
    }
    @Test fun alreadyInsetShellDoesNotAddSafeAreaTwice() {
        assertEquals(SchoolFrame(0, 72, 1080, 2100),
            SchoolFrame.calculate(0.0, 0.0, 360.0, 700.0, 3.0, 0, 72, 0, 72, 1080, 2328))
    }
    @Test fun respectsLandscapeCutoutAndOverlayOrigin() {
        assertEquals(SchoolFrame(80, 100, 1200, 540),
            SchoolFrame.calculate(0.0, 50.0, 600.0, 270.0, 2.0, 80, 0, 80, 0, 1280, 640))
    }
    @Test fun keyboardClipsWithoutNegativeHeight() {
        assertEquals(SchoolFrame(0, 400, 1000, 0),
            SchoolFrame.calculate(0.0, 200.0, 500.0, 600.0, 2.0, 0, 0, 0, 0, 1000, 300))
    }
}
