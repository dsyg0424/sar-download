/** Display-only smoothing; raw transfer metrics remain unchanged. Times are monotonic ms. */
export class TransferSpeed {
  private value = 0;
  private sampledAt: number | null = null;
  private positiveAt: number | null = null;

  sample(raw: number, now: number) {
    const speed = Number.isFinite(raw) ? Math.max(0, raw) : 0;
    const expired = this.positiveAt === null || now - this.positiveAt >= 2000;
    if (speed > 0) this.positiveAt = now;
    if (this.sampledAt === null || expired) this.value = speed;
    else {
      const weight = 1 - Math.exp(-Math.max(0, now - this.sampledAt) / 2000);
      this.value += weight * (speed - this.value);
    }
    this.sampledAt = now;
  }

  read(now: number) {
    return this.positiveAt === null || now - this.positiveAt >= 2000 ? 0 : this.value;
  }
}
