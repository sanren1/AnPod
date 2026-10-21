package app.anpod.receiver;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public class AodDriftTest {

    @Test
    public void driftStaysInsideTheDeclaredTravel() {
        for (int second = 0; second <= 1_200; second++) {
            double elapsed = second;
            assertTrue(Math.abs(AodDrift.xDp(elapsed)) <= AodDrift.MAX_X_DP + 0.001f);
            assertTrue(Math.abs(AodDrift.yDp(elapsed)) <= AodDrift.MAX_Y_DP + 0.001f);
        }
    }

    @Test
    public void driftReachesBothSidesOfThePath() {
        boolean positiveX = false, negativeX = false, positiveY = false, negativeY = false;
        for (int second = 0; second <= 600; second++) {
            double elapsed = second;
            if (AodDrift.xDp(elapsed) > AodDrift.MAX_X_DP * 0.95f) positiveX = true;
            if (AodDrift.xDp(elapsed) < -AodDrift.MAX_X_DP * 0.95f) negativeX = true;
            if (AodDrift.yDp(elapsed) > AodDrift.MAX_Y_DP * 0.95f) positiveY = true;
            if (AodDrift.yDp(elapsed) < -AodDrift.MAX_Y_DP * 0.95f) negativeY = true;
        }
        assertTrue(positiveX && negativeX && positiveY && negativeY);
    }

    @Test
    public void oneSecondStepsStayUnderAPixel() {
        // Slow enough that text never appears to jump between the 1 s updates.
        for (int second = 0; second < 600; second++) {
            assertTrue(Math.abs(AodDrift.xDp(second + 1) - AodDrift.xDp(second)) < 1f);
            assertTrue(Math.abs(AodDrift.yDp(second + 1) - AodDrift.yDp(second)) < 1f);
        }
    }

    @Test
    public void exitLabelHidesAfterIdleAndReturnsOnTouch() {
        assertTrue(AodDrift.exitVisible(0));
        assertTrue(AodDrift.exitVisible(AodDrift.EXIT_IDLE_MILLIS - 1));
        assertFalse(AodDrift.exitVisible(AodDrift.EXIT_IDLE_MILLIS));
        assertFalse(AodDrift.exitVisible(AodDrift.EXIT_IDLE_MILLIS * 4));
    }
}
