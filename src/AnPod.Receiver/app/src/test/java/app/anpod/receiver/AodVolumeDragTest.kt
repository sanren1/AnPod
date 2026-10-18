package app.anpod.receiver

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AodVolumeDragTest {
    @Test
    fun upwardRaisesAndDownwardLowersMediaVolume() {
        val drag = AodVolumeDrag(5, 0, 10, 200f)
        assertEquals(7, drag.move(-40f))
        assertEquals(4, drag.move(60f))
    }

    @Test
    fun smallMovesAccumulateWithoutRepeatingTheSameStep() {
        val drag = AodVolumeDrag(5, 0, 10, 200f)
        assertNull(drag.move(-4f))
        assertNull(drag.move(-4f))
        assertEquals(6, drag.move(-4f))
        assertNull(drag.move(-1f))
    }

    @Test
    fun volumeBoundsDiscardOverscrollSoReversalRespondsImmediately() {
        val drag = AodVolumeDrag(5, 2, 12, 200f)
        assertEquals(12, drag.move(-1000f))
        assertEquals(11, drag.move(20f))
        assertEquals(2, drag.move(1000f))
        assertEquals(3, drag.move(-20f))
    }
}
