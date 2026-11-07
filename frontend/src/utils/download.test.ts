import { describe, it, expect, vi, afterEach } from 'vitest';
import { downloadCsv } from './download';

// jsdom implements neither URL.createObjectURL nor anchor navigation, so the only seam a
// download can be observed at is the object-URL call that hands the Blob over. The Blob is
// then read back as bytes, because the BOM is a byte-level fact: a UTF-8 decode consumes a
// leading one, so reading the Blob back as text cannot tell one BOM from two.
const captureBlob = () => {
  const url = URL as unknown as Record<string, unknown>;
  const saved = { create: url.createObjectURL, revoke: url.revokeObjectURL };
  let blob: Blob | undefined;
  url.createObjectURL = (b: Blob) => {
    blob = b;
    return 'blob:mock-url';
  };
  url.revokeObjectURL = vi.fn();
  const click = vi
    .spyOn(HTMLAnchorElement.prototype, 'click')
    .mockImplementation(() => {});
  return {
    get blob() {
      return blob;
    },
    restore: () => {
      url.createObjectURL = saved.create;
      url.revokeObjectURL = saved.revoke;
      click.mockRestore();
    },
  };
};

const readHeadBytes = (blob: Blob, n: number) =>
  new Promise<number[]>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve([...new Uint8Array(reader.result as ArrayBuffer)]);
    reader.onerror = () => reject(reader.error);
    reader.readAsArrayBuffer(blob.slice(0, n));
  });

const BOM_BYTES = [0xef, 0xbb, 0xbf];

describe('downloadCsv', () => {
  afterEach(() => vi.restoreAllMocks());

  it('prepends a UTF-8 BOM to a BOM-less payload', async () => {
    const dl = captureBlob();
    try {
      downloadCsv('out.csv', 'row_type,transaction_id\r\n');
      // Fourth byte is 'r': the header follows the BOM directly, with nothing between.
      expect(await readHeadBytes(dl.blob as Blob, 4)).toEqual([...BOM_BYTES, 0x72]);
    } finally {
      dl.restore();
    }
  });

  it('does not add a second BOM when the caller already added one', async () => {
    const dl = captureBlob();
    try {
      // The failure this guards: a builder that grows its own BOM would otherwise ship two,
      // and the decoded header would start with U+FEFF for every consumer, Excel included.
      downloadCsv('out.csv', '\uFEFFdate,symbol\r\n');
      // Fourth byte is 'd'.
      expect(await readHeadBytes(dl.blob as Blob, 4)).toEqual([...BOM_BYTES, 0x64]);
    } finally {
      dl.restore();
    }
  });

  it('types the Blob as UTF-8 CSV', async () => {
    const dl = captureBlob();
    try {
      downloadCsv('out.csv', 'a,b\r\n');
      expect(dl.blob?.type).toBe('text/csv;charset=utf-8');
    } finally {
      dl.restore();
    }
  });
});
