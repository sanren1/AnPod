package app.anpod.receiver

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

class AodVolumeDragTest {
    @Test
    fun blockedSystemWriteKeepsOnlyLatestPendingTarget() = runBlocking {
        val targets = AodVolumeTargetQueue()
        val firstWriteStarted = CompletableDeferred<Unit>()
        val releaseFirstWrite = CompletableDeferred<Unit>()
        val applied = mutableListOf<Int>()
        targets.offer(4)

        val writer = launch {
            consumeAodVolumeTargets(targets) { target ->
                applied.add(target)
                if (applied.size == 1) {
                    firstWriteStarted.complete(Unit)
                    releaseFirstWrite.await()
                }
            }
        }

        firstWriteStarted.await()
        targets.offer(5)
        targets.offer(7)
        targets.offer(6)
        targets.close()
        releaseFirstWrite.complete(Unit)
        writer.join()

        assertEquals(listOf(4, 6), applied)
    }

    @Test
    fun nextGestureStartsFromLatestPendingTargetUntilSystemWritesCatchUp() = runBlocking {
        val targets = AodVolumeTargetQueue()
        val firstWriteStarted = CompletableDeferred<Unit>()
        val releaseFirstWrite = CompletableDeferred<Unit>()
        val applied = mutableListOf<Int>()
        var systemVolume = 3
        targets.offer(4)

        val writer = launch {
            consumeAodVolumeTargets(targets) { target ->
                applied.add(target)
                if (applied.size == 1) {
                    firstWriteStarted.complete(Unit)
                    releaseFirstWrite.await()
                }
                systemVolume = target
            }
        }

        firstWriteStarted.await()
        val nextDrag = AodVolumeDrag(targets.startingVolume { systemVolume }, 0, 10, 200f)
        assertEquals(4, targets.startingVolume { systemVolume })
        assertEquals(5, nextDrag.move(-20f))
        targets.offer(5)
        targets.close()
        releaseFirstWrite.complete(Unit)
        writer.join()

        assertEquals(listOf(4, 5), applied)
        systemVolume = 7
        assertEquals(7, targets.startingVolume { systemVolume })
    }

    @Test
    fun canceledWriterDoesNotApplyPendingTargetsAfterInFlightWriteReturns() = runBlocking {
        val targets = AodVolumeTargetQueue()
        val firstWriteStarted = CompletableDeferred<Unit>()
        val releaseFirstWrite = CompletableDeferred<Unit>()
        val applied = mutableListOf<Int>()
        targets.offer(4)

        val writer = launch {
            consumeAodVolumeTargets(targets) { target ->
                applied.add(target)
                if (applied.size == 1) {
                    firstWriteStarted.complete(Unit)
                    withContext(NonCancellable) { releaseFirstWrite.await() }
                }
            }
        }

        firstWriteStarted.await()
        targets.offer(6)
        targets.cancel()
        writer.cancel()
        releaseFirstWrite.complete(Unit)
        writer.join()

        assertEquals(listOf(4), applied)
    }

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
