package top.jxzs.companion;
import org.junit.Test;
import static org.junit.Assert.*;

public class PointerFrameTest {
    @Test public void normalizedHotspotWorksAcrossPhoneAndTabletSizes() {
        PointerFrame f = new PointerFrame(1, true, 0.25, 0.75, 1920, 1080, 100);
        assertEquals(480, f.screenX(1920)); assertEquals(810, f.screenY(1080));
        assertEquals(640, f.screenX(2560)); assertEquals(1080, f.screenY(1440));
        assertTrue(f.canShow(150, 2560, 1440));
    }
    @Test public void edgeHotspotRemainsOnscreen() {
        PointerFrame f = new PointerFrame(1, true, 1, 1, 1080, 2400, 100);
        assertEquals(1079, f.screenX(1080)); assertEquals(2399, f.screenY(2400));
    }
    @Test public void disconnectHideAndRotationMismatchAreFailClosed() {
        PointerFrame f = new PointerFrame(1, true, 0.5, 0.5, 1080, 2400, 100);
        assertTrue(f.canShow(500, 1080, 2400));
        assertFalse(f.canShow(801, 1080, 2400));
        assertFalse(f.canShow(150, 2400, 1080));
        assertFalse(new PointerFrame(1, false, 0.5, 0.5, 1080, 2400, 100).canShow(110, 1080, 2400));
    }
    @Test(expected = IllegalArgumentException.class) public void rejectNaN() { new PointerFrame(1, true, Double.NaN, 0, 1, 1, 0); }
    @Test(expected = IllegalArgumentException.class) public void rejectUnknownProtocol() { new PointerFrame(2, true, 0, 0, 1, 1, 0); }
    @Test(expected = IllegalArgumentException.class) public void rejectInvalidBounds() { new PointerFrame(1, true, -0.1, 0, 1, 1, 0); }
}
