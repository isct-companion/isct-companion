package app.isctcompanion.core
import java.io.File
import kotlinx.coroutines.runBlocking

class MemoryVault : NativeVault {
    private val values = mutableMapOf<String,String>()
    private var failStore = false
    @Synchronized fun failNextStore() { failStore = true }
    @Synchronized override fun load(account:String):String? = values[account]
    @Synchronized override fun store(account:String,secret:String) {
        if(failStore) { failStore=false; throw IllegalStateException("native-vault-fixture") }
        values[account]=secret
    }
    @Synchronized override fun delete(account:String) { values.remove(account) }
}
fun main(args:Array<String>) = runBlocking {
    val directory=if(args.isNotEmpty())File(args[0]) else File(System.getProperty("java.io.tmpdir"),"isct-kotlin-${System.nanoTime()}")
    val config=SessionConfig(File(directory,"data").absolutePath,File(directory,"files").absolutePath,SyncNetwork.OFFLINE,listOf(),listOf())
    val vault=MemoryVault()
    val session=Session.open(config,vault,null,null,null,null)
    val logs=exportApplicationLogs("smoke")
    check(logs.startsWith("smoke\n") && logs.contains("Application started")) { "Opening a session must install Core's logger" }
    check(schoolBrowserInitialization().isNotEmpty())
    val credentials=SchoolCredentials("fixture","not-a-real-password","","A".repeat(70))
    vault.failNextStore()
    try {session.saveCredentials(SchoolAuthProfile.TOKYO_TECH,credentials).value();error("Missing callback error")}
    catch(_:SdkException.Callback) {}
    session.saveCredentials(SchoolAuthProfile.TOKYO_TECH,credentials).value()
    session.deleteCredentials(SchoolAuthProfile.TOKYO_TECH).value()
    val stream=session.subscribe()
    val first=(session.snapshot().value() as OperationResult.Workspace).snapshot
    val op=session.mutate(WorkspaceMutation(first.revision,WorkspaceIntent.SetAdmissionYear(2026)))
    check(!op.cancel())
    val committed=(op.value() as OperationResult.Workspace).snapshot
    check(first.revision!=committed.revision)
    try {session.mutate(WorkspaceMutation(first.revision,WorkspaceIntent.SetAdmissionYear(2025))).value();error("Missing conflict")}
    catch(_:SdkException.Conflict) {}
    check(stream.next() is StreamUpdate.Event)
    val queries=listOf(WorkspaceQuery.Glance("2026-10-15T09:00:00+09:00"),WorkspaceQuery.NotificationPlan("2026-10-15T09:00:00+09:00","2026-10-16T09:00:00+09:00",null))
    val result=(session.scheduleQuery(queries).value() as OperationResult.Schedule).snapshot
    check(result.revision==committed.contentRevision && result.values.size==2)
    val clockResult=(session.scheduleQuery(listOf(WorkspaceQuery.TeachingPeriods)).value() as OperationResult.Schedule).snapshot
    val periods=(clockResult.values.single() as WorkspaceQueryResult.TeachingPeriods).v1
    check(periods.size==6 && periods[5].periods.contentEquals(byteArrayOf(11,12)) && periods[5].startTime=="19:05" && periods[5].endTime=="20:45")
    session.parseLmsResource("<html></html>","https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=1","resource").value()
    session.exportBackup().value()
    if(session.capabilities().ocr) {
        session.prepareScanner().value()
        try {session.scanMatrix(byteArrayOf(1,2,3)).value();error("Expected invalid image")}
        catch(_:SdkException.InvalidInput) {}
    } else {
        try {session.scanMatrix(byteArrayOf()).value();error("Expected unavailable")}
        catch(_:SdkException.Unavailable) {}
    }
    session.suspend().value();session.resume().value()
    stream.unsubscribe()
    try {stream.next();error("Expected closed stream")}catch(_:SdkException.Closed){}
    session.shutdown();session.shutdown()
    try {session.snapshot();error("Expected closed")}catch(_:SdkException.Closed){}
    session.close();stream.close();directory.deleteRecursively()
    println("KOTLIN_CORE_SDK_SMOKE_PASSED")
}
