import Foundation
import IsctCore
import Observation

/// One active read for a set of views. Core owns the dependency versions;
/// a forced read also invalidates earlier answers at the same storage version.
@MainActor
final class ScheduleReads {
    private(set) var generation = 0
    private var publishedGeneration = -1
    private var reads: [WorkspaceChange] = []
    private var publishedSeq: UInt64 = 0
    private(set) var publishedVersion: UInt64 = 0
    private var task: Task<Void, Never>?

    func invalidate() { generation += 1 }

    func current(_ session: Session) throws -> Bool {
        guard publishedGeneration == generation else { return false }
        return try session.readVersion(reads: reads) <= publishedSeq
    }

    func accept(_ snapshot: ScheduleSnapshot, started: Int, session: Session) throws -> Bool {
        guard started == generation, snapshot.seq >= publishedSeq else { return false }
        let version = try session.readVersion(reads: snapshot.reads)
        guard version <= snapshot.seq else { return false }
        publishedGeneration = generation
        publishedSeq = snapshot.seq
        publishedVersion = version
        reads = snapshot.reads
        return true
    }

    func refresh(_ session: Session, read: @escaping (Int) async throws -> Void,
                 failed: @escaping (Error) -> Void) -> Task<Void, Never> {
        if let task { return task }
        let next = Task {
            defer { task = nil }
            do {
                while try !current(session) { try await read(generation) }
            } catch { failed(error) }
        }
        task = next
        return next
    }
}

/// The pager keeps its last answer for each Monday while Core recomputes it.
@MainActor @Observable
final class TimetableWeeks {
    private let reads = ScheduleReads()
    private var shown: [String: TimetablePage] = [:]
    @ObservationIgnored private var fresh: Set<String> = []
    private(set) var revision = 0

    subscript(monday: Date) -> TimetablePage? { shown[AppModel.civil(monday)] }

    /// A week on screen reads again when a commit touched what it read, or
    /// when its last read did not finish.
    func invalidateIfChanged(_ session: Session) {
        guard !shown.isEmpty else { return }
        if fresh.isEmpty || (try? reads.current(session)) == false { invalidate() }
    }

    func invalidate(discard: Bool = false) {
        reads.invalidate()
        fresh.removeAll()
        revision += 1
        if discard { shown.removeAll() }
    }

    func week(starting monday: Date, session: Session) async -> TimetablePage? {
        let key = AppModel.civil(monday)
        if fresh.contains(key), (try? reads.current(session)) == true { return shown[key] }

        let generation = reads.generation
        // A cold read fills the swipe window. Refreshes cover the pages visible while swiping.
        let reach: UInt8 = shown[key] == nil ? 4 : 1
        let query = WorkspaceQuery.timetableWeeks(monday: key, reach: reach, mode: .classes)

        while !Task.isCancelled, generation == reads.generation {
            guard case let .schedule(snapshot)? = try? await session.scheduleQuery(queries: [query]).value(),
                  case let .timetableWeeks(weeks)? = snapshot.values.first,
                  !Task.isCancelled, generation == reads.generation else { return nil }
            let previousVersion = reads.publishedVersion
            guard let accepted = try? reads.accept(snapshot, started: generation, session: session) else { return nil }
            if !accepted { continue }

            // A commit may already be visible to Core before its event arrives.
            if reads.publishedVersion != previousVersion { fresh.removeAll() }
            let loaded = Dictionary(uniqueKeysWithValues: weeks.map { ($0.monday, $0) })
            shown.merge(loaded) { _, new in new }
            fresh.formUnion(loaded.keys)
            return shown[key]
        }
        return nil
    }
}
