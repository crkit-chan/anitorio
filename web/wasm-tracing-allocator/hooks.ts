type AllocatorSize = number;
type AllocatorAlign = number;
type AllocatorPointer = number;

export interface AllocatorEvent {
  size: AllocatorSize;
  align: AllocatorAlign;
  pointer: AllocatorPointer;
  getStack(): string | undefined;
}

export interface ReallocEvent {
  oldPointer: AllocatorPointer;
  newPointer: AllocatorPointer;
  oldSize: AllocatorSize;
  newSize: AllocatorSize;
  align: AllocatorAlign;
  getStack(): string | undefined;
}

export interface AllocatorListeners {
  alloc(event: AllocatorEvent): void;
  allocZeroed(event: AllocatorEvent): void;
  dealloc(event: AllocatorEvent): void;
  realloc(event: ReallocEvent): void;
}

type AllocatorEventType = keyof AllocatorListeners;

type Listener<T> = (event: T) => void;

const listeners: { [K in AllocatorEventType]: Set<AllocatorListeners[K]> } = {
  alloc: new Set(),
  dealloc: new Set(),
  allocZeroed: new Set(),
  realloc: new Set(),
};

function emit<K extends AllocatorEventType>(type: K, event: Omit<Parameters<AllocatorListeners[K]>[0], "getStack">): void {
  for (const listener of listeners[type]) {
    (listener as Listener<typeof event>)({ ...event, getStack });
  }
}

export function addAllocatorListener<K extends AllocatorEventType>(type: K, listener: AllocatorListeners[K]): () => void {
  listeners[type].add(listener);

  return () => listeners[type].delete(listener);
}

export class Allocation {
  align: AllocatorAlign;
  pointer: AllocatorPointer;
  size: AllocatorSize;
  stack?: string;

  constructor(size: AllocatorSize, align: AllocatorAlign, pointer: AllocatorPointer) {
    this.size = size;
    this.align = align;
    this.pointer = pointer;
    this.stack = getStack();
  }
}
export class InvalidFree extends Allocation {}

const liveAllocs = new Map<AllocatorPointer, Allocation>();
const invalidFrees = new Map<AllocatorPointer, InvalidFree>();

let totalAllocated: AllocatorSize = 0;
export function getTotalAllocated() {
  return totalAllocated;
}

function getStack(): string | undefined {
  const lines = Error().stack?.split("\n");
  return lines?.filter((frame) => !/hooks\.js/.test(frame)).join("\n");
}

function onAlloc(size: AllocatorSize, align: AllocatorAlign, pointer: AllocatorPointer): void {
  totalAllocated += size;
  liveAllocs.set(pointer, new Allocation(size, align, pointer));

  emit("alloc", { size, align, pointer });
}

function onDealloc(size: AllocatorSize, align: AllocatorAlign, pointer: AllocatorPointer): void {
  totalAllocated -= size;
  const wasLive = liveAllocs.delete(pointer);
  if (!wasLive) {
    invalidFrees.set(pointer, new InvalidFree(size, align, pointer));
  }

  emit("dealloc", { size, align, pointer });
}

function onAllocZeroed(size: AllocatorSize, align: AllocatorAlign, pointer: AllocatorPointer): void {
  onAlloc(size, align, pointer);

  emit("allocZeroed", { size, align, pointer });
}

function onRealloc(
  oldPointer: AllocatorPointer,
  newPointer: AllocatorPointer,
  oldSize: AllocatorSize,
  newSize: AllocatorSize,
  align: AllocatorAlign,
): void {
  onDealloc(oldSize, align, oldPointer);
  onAlloc(newSize, align, newPointer);

  emit("realloc", { oldPointer, newPointer, oldSize, newSize, align });
}

interface DumpTableOptions<T> {
  keyLabel: string;
  valueLabel: string;
  getKey(entry: T): string | number;
  getValue(entry: T): number;
}

export function dumpTable<T>(entries: Iterable<T>, { keyLabel, valueLabel, getKey, getValue }: DumpTableOptions<T>): void {
  const byKey = new Map<string | number, number>();
  let total = 0;

  for (const entry of entries) {
    const key = getKey(entry);
    const entryValue = getValue(entry);

    total += entryValue;
    byKey.set(key, (byKey.get(key) || 0) + entryValue);
  }

  const table = [...byKey]
    .sort((a, b) => b[1] - a[1])
    .map(([k, v]) => ({
      [valueLabel]: v,
      [keyLabel]: k,
    }));

  table.unshift({ [valueLabel]: total, [keyLabel]: "<total>" });
  console.table(table, [keyLabel, valueLabel]);
}

type DumpOverrides<T> = Partial<DumpTableOptions<T>>;

const defaultLiveOptions: DumpTableOptions<Allocation> = {
  keyLabel: "Live Allocations",
  valueLabel: "Size (Bytes)",
  getKey: (entry) => entry.stack ?? "<unknown>",
  getValue: () => 1,
};

const defaultInvalidOptions: DumpTableOptions<InvalidFree> = {
  keyLabel: "Invalid Free",
  valueLabel: "Count",
  getKey: (entry) => entry.stack ?? "<unknown>",
  getValue: () => 1,
};

export const WasmTracingAllocator = {
  on_alloc: onAlloc,
  on_dealloc: onDealloc,
  on_alloc_zeroed: onAllocZeroed,
  on_realloc: onRealloc,

  addListener: addAllocatorListener,
  getTotalAllocated,

  dumpLiveAllocations(opts: DumpOverrides<Allocation> = {}): void {
    dumpTable(liveAllocs.values(), { ...defaultLiveOptions, ...opts });
  },
  dumpInvalidFrees(opts: DumpOverrides<InvalidFree> = {}): void {
    dumpTable(invalidFrees.values(), { ...defaultInvalidOptions, ...opts });
  },
};

(globalThis as any).WasmTracingAllocator = WasmTracingAllocator;
