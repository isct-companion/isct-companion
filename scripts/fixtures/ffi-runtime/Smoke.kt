package uniffi.isct_ffi_runtime_spike

import kotlinx.coroutines.*
import java.util.Collections

class Listener(private val fails: Boolean = false): ProbeListener {
    val requests = Collections.synchronizedList(mutableListOf<ULong>())
    override fun completed(request: ULong) {
        if (fails) throw IllegalArgumentException("foreign callback failure")
        requests.add(request)
    }
}
fun main() = runBlocking {
    val probe = Probe()
    val listener = Listener()
    check(probe.run(7uL, 5uL, listener) == 7uL)
    check(listener.requests == listOf(7uL))
    try {
        probe.run(8uL, 1uL, Listener(true))
        error("foreign exception was swallowed")
    } catch (_: ProbeException.Callback) { }
    val cancelledListener = Listener()
    val pending = launch { probe.run(9uL, 1000uL, cancelledListener) }
    delay(20)
    pending.cancelAndJoin()
    withTimeout(2000) { while (probe.pending() != 0uL) delay(10) }
    check(cancelledListener.requests.isEmpty())
    val explicit = async { runCatching { probe.run(10uL, 1000uL, listener) } }
    withTimeout(2000) { while (probe.pending() == 0uL) delay(10) }
    check(probe.cancel(10uL))
    check(explicit.await().exceptionOrNull() is ProbeException.Cancelled)
    val closing = async { runCatching { probe.run(12uL, 1000uL, listener) } }
    withTimeout(2000) { while (probe.pending() == 0uL) delay(10) }
    probe.shutdown()
    check(closing.await().exceptionOrNull() is ProbeException.Cancelled)
    check(listener.requests == listOf(7uL))
    probe.shutdown()
    try {
        probe.run(11uL, 1uL, listener)
        error("run after shutdown succeeded")
    } catch (_: ProbeException.Closed) { }
    try { injectedPanic(); error("panic was swallowed") }
    catch (error: InternalException) { check(error.message!!.contains("FFI smoke-test panic")) }
    check(FfiConverterTypeProbeListener.handleMap.size == 0)
    probe.close()
    check(probe.uniffiIsDestroyed)
    println("KOTLIN PASS: runtime, correlation, foreign error, cancellation, shutdown, panic")
}
