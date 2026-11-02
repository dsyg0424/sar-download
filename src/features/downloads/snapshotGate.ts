import { mergeSnapshot, type DownloadTask } from "./api";

// Request ordering and event ordering are separate: a late snapshot cannot erase
// an event received after its request started, even for a previously unseen ID.
export class SnapshotGate {
  private request = 0;
  private event = 0;
  private touched = new Map<string, number>();
  begin() { return { request: ++this.request, event: this.event }; }
  touch(id: string) { this.touched.set(id, ++this.event); }
  invalidate() { ++this.request; }
  merge(current: DownloadTask[], incoming: DownloadTask[], ticket: ReturnType<SnapshotGate["begin"]>, removed: Set<string>) {
    if (ticket.request !== this.request) return current.filter(t => !removed.has(t.id));
    const protectedIds = new Set([...this.touched].filter(([,stamp]) => stamp > ticket.event).map(([id]) => id));
    const result = mergeSnapshot(current, incoming, protectedIds).filter(t => !removed.has(t.id));
    return result;
  }
}
