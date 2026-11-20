import Foundation
import IsctCore

/// One active read for a set of views. Core owns the dependency versions;
/// a forced read also invalidates earlier answers at the same storage version.
@MainActor
final class ScheduleReads {
    private(set) var generation = 0
    private var publishedGeneration = -1
    private var reads: [WorkspaceChange] = []
    private var publishedSeq: UInt64 = 0
    private var task: Task<Void, Never>?

    func invalidate() { generation += 1 }

    func current(_ session: Session) throws -> Bool {
        guard publishedGeneration == generation else { return false }
        return try session.readVersion(reads: reads) <= publishedSeq
    }

    func accept(_ snapshot: ScheduleSnapshot, started: Int, session: Session) throws -> Bool {
        guard started == generation, snapshot.seq >= publishedSeq,
              try session.readVersion(reads: snapshot.reads) <= snapshot.seq else { return false }
        publishedGeneration = generation
        publishedSeq = snapshot.seq
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
