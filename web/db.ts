import { SignalPreset } from "@/data";
import { BASE_URL } from "@/utils";
import { gunzipSync, gzipSync } from "fflate";

const DB_VERSION = 3;

const APP_STORAGE_KEY = "app-storage";
const CHUNK_STORAGE_KEY = "chunk-storage";
const FILE_STORAGE_KEY = "file-storage";
const SIGNAL_PRESETS_STORAGE_KEY = "signal-data-presets";

const CHUNK_SIZE = 32 * 1024 * 1024;

const chunkKey = (key: string, index: number) => [key, "chunk", index];

const PRESET_MAP = {
  "compat-v2.0.77": { url: `${BASE_URL}assets/data/presets/compat-v2.0.77.json` },
  "dlc-v2.0.77": { url: `${BASE_URL}assets/data/presets/space-age-v2.0.77.json` },
} as const;
export const DEFAULT_SIGNAL_PRESET_KEYS = Object.keys(PRESET_MAP);

const fileDBPromise: Promise<IDBDatabase> = new Promise((resolve, reject) => {
  const req = indexedDB.open(APP_STORAGE_KEY, DB_VERSION);
  req.onupgradeneeded = () => {
    const { result } = req;
    for (const key of [CHUNK_STORAGE_KEY, FILE_STORAGE_KEY, SIGNAL_PRESETS_STORAGE_KEY]) {
      if (result.objectStoreNames.contains(key)) continue;
      result.createObjectStore(key);
    }
  };
  req.onsuccess = () => resolve(req.result);
  req.onerror = () => reject(req.error);
});

function putDB(store: string, value: unknown, key: IDBValidKey): Promise<void> {
  return fileDBPromise.then((db) => {
    const start = performance.now();

    const tx = db.transaction(store, "readwrite");
    tx.objectStore(store).put(value, key);

    return new Promise((resolve, reject) => {
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  });
}

export async function saveFileDB(file: File | null, key: string): Promise<void> {
  const db = await fileDBPromise;
  return new Promise((resolve, reject) => {
    const tx = db.transaction(FILE_STORAGE_KEY, "readwrite");
    tx.objectStore(FILE_STORAGE_KEY).put(file, key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

export async function loadFileDB(key: string): Promise<File | null> {
  const db = await fileDBPromise;
  return new Promise((resolve, reject) => {
    try {
      const req = db.transaction(FILE_STORAGE_KEY, "readonly").objectStore(FILE_STORAGE_KEY).get(key);
      req.onsuccess = () => resolve(req.result ?? null);
      req.onerror = () => reject(req.error);
    } catch (e) {
      resolve(null);
    }
  });
}

export async function getSignalPresetNames(): Promise<string[]> {
  const db = await fileDBPromise;
  return new Promise((resolve, reject) => {
    const tx = db.transaction(SIGNAL_PRESETS_STORAGE_KEY, "readonly");
    const req = tx.objectStore(SIGNAL_PRESETS_STORAGE_KEY).getAllKeys();
    req.onsuccess = () => resolve(req.result as string[]);
    req.onerror = () => reject(tx.error);
  });
}
export async function addSignalPreset(name: string, preset: SignalPreset): Promise<void> {
  const db = await fileDBPromise;
  return new Promise((resolve, reject) => {
    const tx = db.transaction(SIGNAL_PRESETS_STORAGE_KEY, "readwrite");
    const compressed = gzipSync(new TextEncoder().encode(JSON.stringify(preset)), { level: 9 });
    const req = tx.objectStore(SIGNAL_PRESETS_STORAGE_KEY).put(compressed, name);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(req.error);
  });
}
export async function getSignalPreset(name: string): Promise<{ preset: SignalPreset; raw: Uint8Array }> {
  if (name in PRESET_MAP) {
    const { url } = PRESET_MAP[name as keyof typeof PRESET_MAP];
    const req = await fetch(url);
    return {
      preset: await req.clone().json(),
      raw: new Uint8Array(await req.arrayBuffer()),
    };
  }

  const db = await fileDBPromise;
  return new Promise((resolve, reject) => {
    try {
      const tx = db.transaction(SIGNAL_PRESETS_STORAGE_KEY, "readonly");
      const req = tx.objectStore(SIGNAL_PRESETS_STORAGE_KEY).get(name);
      req.onsuccess = () => {
        try {
          const raw = gunzipSync(req.result);
          const preset: SignalPreset = JSON.parse(new TextDecoder().decode(raw));
          resolve({ raw, preset });
        } catch (e) {
          reject(e);
        }
      };
      req.onerror = () => reject(req.error);
    } catch (e) {
      reject(e);
    }
  });
}
export async function removeSignalPreset(name: string): Promise<void> {
  const db = await fileDBPromise;
  return new Promise((resolve, reject) => {
    const tx = db.transaction(SIGNAL_PRESETS_STORAGE_KEY, "readwrite");
    const req = tx.objectStore(SIGNAL_PRESETS_STORAGE_KEY).delete(name);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(req.error);
  });
}

export class DefaultFileDBChunkStore {
  index: number;

  protected readonly CHUNK_KEY = CHUNK_STORAGE_KEY;

  constructor(public readonly key: string) {
    this.index = 0;
  }

  async deleteAllChunks(): Promise<void> {
    const db = await fileDBPromise;

    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(this.CHUNK_KEY, "readwrite");
      const store = tx.objectStore(this.CHUNK_KEY);
      const range = IDBKeyRange.bound([this.key, "chunk"], [this.key, "chunk", []], true, true);
      const req = store.openCursor(range);

      req.onsuccess = () => {
        const cursor = req.result;
        if (!cursor) return;
        cursor.delete();
        cursor.continue();
      };

      tx.oncomplete = () => {
        this.index = 0;
        resolve();
      };
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  }
  async *readAllChunks() {
    const db = await fileDBPromise;

    for (let i = 0; ; i++) {
      const blob: Blob | undefined = await new Promise((resolve, reject) => {
        const req = db.transaction(this.CHUNK_KEY).objectStore(this.CHUNK_KEY).get(chunkKey(this.key, i));

        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      });

      if (!blob) break;

      const reader = blob.stream().getReader();

      try {
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          yield value;
        }
      } finally {
        reader.releaseLock();
      }
    }
  }

  async writeChunk(chunk: Blob): Promise<void> {
    navigator.storage.estimate().then(({ usage, quota }) => {
      console.log({
        usage: (usage! / 1024 ** 3).toFixed(2) + " GiB",
        quota: (quota! / 1024 ** 3).toFixed(2) + " GiB",
        available: ((quota! - usage!) / 1024 ** 3).toFixed(2) + " GiB",
      });
    });

    await putDB(this.CHUNK_KEY, chunk, chunkKey(this.key, this.index++));
  }
}

export class ChunkWriter {
  private chunks: Uint8Array<ArrayBuffer>[] = [];
  private size = 0;

  constructor(
    private readonly onChunk: (blob: Blob) => void | Promise<void>,
    private readonly chunkSize: number = CHUNK_SIZE,
  ) {}

  async write(data: Uint8Array): Promise<void> {
    let offset = 0;

    while (offset < data.length) {
      const length = Math.min(this.chunkSize - this.size, data.length - offset);

      this.chunks.push(data.slice(offset, offset + length));
      this.size += length;
      offset += length;

      if (this.size === this.chunkSize) {
        await this.flush();
      }
    }
  }

  async flush(): Promise<void> {
    if (this.size === 0) return;

    await this.onChunk(new Blob(this.chunks));

    this.chunks = [];
    this.size = 0;
  }
}
