import type { DictationEvent, PendingResult } from "./types";

/**
 * A terminal STT failure must not reuse an older pending result in the banner.
 * A successful paste or cancellation, however, must leave an older queued
 * recovery visible until the user handles it.
 */
export function visiblePendingResult(
  pending: PendingResult | null,
  event: DictationEvent | null,
): PendingResult | null {
  if (!pending) return null;
  if (!event || event.phase === "processing") {
    return event?.phase === "processing" ? null : pending;
  }
  if (event.recovery_available) return pending;
  return event.outcome === "pasted" || event.outcome === "cancelled" ? pending : null;
}
