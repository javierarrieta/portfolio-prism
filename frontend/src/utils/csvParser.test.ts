import { describe, test, expect } from 'vitest';
import Papa from 'papaparse';
import { detectColumns, detectDateFormat } from './csvParser';

describe('detectColumns', () => {
  test('maps exact column names', () => {
    const result = detectColumns(['date', 'symbol', 'type', 'quantity', 'price', 'fee']);
    expect(result[0]).toBe('date');
    expect(result[1]).toBe('symbol');
    expect(result[2]).toBe('type');
    expect(result[3]).toBe('quantity');
    expect(result[4]).toBe('price');
    expect(result[5]).toBe('fee');
  });

  test('maps case-insensitive column names', () => {
    const result = detectColumns(['Date', 'Symbol', 'Type', 'Qty', 'Price', 'Fee']);
    expect(result[0]).toBe('date');
    expect(result[1]).toBe('symbol');
    expect(result[2]).toBe('type');
    expect(result[3]).toBe('quantity');
    expect(result[4]).toBe('price');
    expect(result[5]).toBe('fee');
  });

  test('maps underscores and camelCase', () => {
    const result = detectColumns(['txn_date', 'unit_price', 'ticker']);
    expect(result[0]).toBe('date');
    expect(result[1]).toBe('price');
    expect(result[2]).toBe('symbol');
  });

  test('maps unknown columns to null', () => {
    const result = detectColumns(['date', 'description', 'symbol']);
    expect(result[0]).toBe('date');
    expect(result[1]).toBeNull();
    expect(result[2]).toBe('symbol');
  });

  test('returns empty mapping for empty headers', () => {
    const result = detectColumns([]);
    expect(Object.keys(result)).toHaveLength(0);
  });

  test('does not duplicate mappings for repeated field types', () => {
    const result = detectColumns(['date', 'txn_date', 'symbol', 'ticker', 'qty']);
    const values = Object.values(result).filter((v) => v !== null);
    const uniqueValues = new Set(values);
    expect(uniqueValues.size).toBe(values.length);
  });

  test('maps first occurrence of duplicated field type', () => {
    const result = detectColumns(['date', 'datetime', 'symbol']);
    expect(result[0]).toBe('date');
    expect(result[1]).toBeNull();
    expect(result[2]).toBe('symbol');
  });
});

// Excel writes a BOM when you save CSV from a UTF-8 workbook, so every Excel-authored
// upload starts with U+FEFF. Two layers swallow it: Papa strips it before the header row is
// built (Papa.BYTE_ORDER_MARK / stripBom), and detectColumns' `[^a-z0-9_]` sanitizer drops it
// a second time. What this file owns is the seam between them -- `meta.fields` feeds both
// detectColumns and every `row[h]` lookup after it -- so that is what gets pinned. Papa's own
// strip is a third-party guarantee, pinned here only so a bump that changes it fails in CI
// rather than on a user's upload, where the symptom is `Symbol is required` on every row and
// points at their data instead of at the parser.
describe('BOM on upload', () => {
  // CsvImportModal parses the uploaded File, not a string, and Papa's BOM strip is on a
  // different code path for each (stripBom on the string input, stripBom per header on the
  // chunked FileReader route). Pin the path the app actually uses.
  const parseFile = (csv: string) =>
    new Promise<Papa.ParseResult<Record<string, string>>>((resolve, reject) => {
      Papa.parse(new File([csv], 'transactions.csv', { type: 'text/csv' }), {
        header: true,
        skipEmptyLines: true,
        complete: (results) => resolve(results as Papa.ParseResult<Record<string, string>>),
        error: (err) => reject(err),
      });
    });

  test('a BOM-prefixed upload maps every column and keeps its data', async () => {
    const result = await parseFile(
      '\uFEFFdate,symbol,type,quantity,price,fee\r\n2024-01-02,AAPL,buy,10,100,1\r\n'
    );
    const headers = result.meta.fields ?? [];

    expect(headers[0]?.codePointAt(0)).not.toBe(0xfeff);
    expect(headers).toEqual(['date', 'symbol', 'type', 'quantity', 'price', 'fee']);

    const mapping = detectColumns(headers);
    expect(mapping[0]).toBe('date');
    expect(mapping[1]).toBe('symbol');
    // The row keys come from the same header strings, so a BOM surviving into `headers`
    // would also break every row[h] lookup downstream (detectDateFormat, the confirm step).
    expect(result.data[0]?.symbol).toBe('AAPL');
    expect(result.data[0]?.[headers[0]]).toBe('2024-01-02');
  });

  test('detectColumns tolerates a BOM even if one reaches it', () => {
    expect(detectColumns(['\uFEFFdate', '\uFEFFsymbol'])[0]).toBe('date');
    expect(detectColumns(['\uFEFFdate', '\uFEFFsymbol'])[1]).toBe('symbol');
  });
});

describe('detectDateFormat', () => {
  test('detects YYYY-MM-DD format', () => {
    const result = detectDateFormat([['2024-01-15', 'AAPL', 'BUY', '100', '150.00', '5.00']]);
    expect(result.format).toBe('YYYY-MM-DD');
    expect(result.ambiguous).toBe(false);
  });

  test('detects MM/DD/YYYY format', () => {
    const result = detectDateFormat([['01/15/2024', 'AAPL', 'BUY', '100', '150.00']]);
    expect(result.format).toBe('MM/DD/YYYY');
    expect(result.ambiguous).toBe(true);
  });

  test('detects DD/MM/YYYY format', () => {
    const result = detectDateFormat([['15/01/2024', 'AAPL', 'BUY', '100', '150.00']]);
    expect(result.format).toBe('DD/MM/YYYY');
    expect(result.ambiguous).toBe(true);
  });

  test('detects YYYY-MM-DD HH:MM format', () => {
    const result = detectDateFormat([['2024-01-15 10:30', 'AAPL', 'BUY', '100', '150.00']]);
    expect(result.format).toBe('YYYY-MM-DD HH:MM');
    expect(result.ambiguous).toBe(false);
  });

  test('defaults to YYYY-MM-DD for non-matching cells', () => {
    const result = detectDateFormat([['hello', 'world', 'test']]);
    expect(result.format).toBe('YYYY-MM-DD');
    expect(result.ambiguous).toBe(false);
  });

  test('returns first matching format when multiple date cells exist', () => {
    const result = detectDateFormat([['2024-01-15', '01/15/2024', 'AAPL']]);
    expect(result.format).toBe('YYYY-MM-DD');
  });
});
