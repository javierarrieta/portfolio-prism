import { describe, test, expect } from 'vitest';
import type { ColumnMapping } from '../types/csv';
import { detectColumns } from './csvParser';
import { validateCsvRow, validateCsvFile } from './csvValidator';

const columnMapping = detectColumns(['date', 'symbol', 'type', 'qty', 'price', 'fee']);

describe('validateCsvRow', () => {
  test('valid row passes with no errors', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00', fee: '5.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.symbol).toBe('AAPL');
    expect(result.type).toBe('BUY');
    expect(result.quantity).toBe(100);
    expect(result.price).toBe(150);
    expect(result.fee).toBe(5);
  });

  test('fee defaults to 0 when missing', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.fee).toBe(0);
  });

  test('case-insensitive type accepted', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'buy', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.type).toBe('BUY');
  });

  test('SELL type accepted', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'SELL', qty: '50', price: '200.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.type).toBe('SELL');
  });

  test('SPLIT type accepted with fee coerced to 0', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'SPLIT', qty: '2', price: '1', fee: '5.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.type).toBe('SPLIT');
    expect(result.fee).toBe(0);
  });

  test('SPLIT with zero price rejected', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'SPLIT', qty: '2', price: '0' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'price')).toBe(true);
  });

  test('SPLIT with non-positive numerator (qty 0) rejected', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'SPLIT', qty: '0', price: '1' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'quantity')).toBe(true);
  });

  test('rejects invalid date format', () => {
    const row = { date: 'not-a-date', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'date')).toBe(true);
  });

  test('rejects non-positive quantity', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '-5', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'quantity')).toBe(true);
  });

  test('rejects negative price', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '-50.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'price')).toBe(true);
  });

  test('rejects invalid type', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'HOLD', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'type')).toBe(true);
  });

  test('rejects missing symbol', () => {
    const row = { date: '2024-01-15', symbol: '', type: 'BUY', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'symbol')).toBe(true);
  });

  test('rejects negative fee', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00', fee: '-5' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'fee')).toBe(true);
  });

  test('row number is correctly assigned', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 5);
    expect(result.row).toBe(5);
  });

  test('accepts zero price for dividend DRIP transactions', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '0' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.price).toBe(0);
  });

  test('accepts DD/MM/YYYY dates where day > 12', () => {
    const row = { date: '15/01/2024', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors.some((e) => e.field === 'date')).toBe(false);
  });

  test('parses price with currency symbol', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '$4.55' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.price).toBe(4.55);
  });

  test('parses price with commas as thousands separator', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '1,234.56' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.price).toBe(1234.56);
  });

  test('parses fee with currency symbol', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00', fee: '$5.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.fee).toBe(5);
  });

  test('parses quantity with currency symbol (tolerated)', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '€100', price: '150.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.quantity).toBe(100);
  });

  test('parses price with pound symbol', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '£4.55' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.price).toBeCloseTo(4.55);
  });

  test('parses price with yen symbol', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '¥150' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.price).toBe(150);
  });

  test('parses price with rupee symbol', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '₹1,234.56' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.price).toBeCloseTo(1234.56);
  });

  test('parses fee with currency symbol and comma separator', () => {
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00', fee: '€1,200.00' };
    const result = validateCsvRow(row, columnMapping, 1);
    expect(result.errors).toHaveLength(0);
    expect(result.fee).toBeCloseTo(1200);
  });

  test('uses constants for unmapped fields', () => {
    // ColumnMapping, not Record<number, string>: the values are the CsvTransactionField
    // union, and a bare string here describes a mapping production could never produce.
    const mapping: ColumnMapping = { 0: 'date', 1: 'quantity', 2: 'price' };
    const row = { date: '2024-01-15', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, mapping, 1, { symbol: 'AAPL', type: 'BUY' });
    expect(result.errors).toHaveLength(0);
    expect(result.symbol).toBe('AAPL');
    expect(result.type).toBe('BUY');
  });

  test('constants do not override mapped CSV columns', () => {
    const mapping = detectColumns(['date', 'symbol', 'type', 'qty', 'price']);
    const row = { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00' };
    const result = validateCsvRow(row, mapping, 1, { symbol: 'MSFT' });
    expect(result.symbol).toBe('AAPL');
  });
});

describe('validateCsvFile', () => {
  test('all valid rows pass validation', () => {
    const rows = [
      { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00' },
      { date: '2024-02-20', symbol: 'MSFT', type: 'BUY', qty: '50', price: '300.00' },
    ];
    const report = validateCsvFile(rows, columnMapping, ['AAPL', 'MSFT']);
    expect(report.valid).toBe(true);
    expect(report.totalRows).toBe(2);
    expect(report.validRows).toBe(2);
    expect(report.errorRows).toHaveLength(0);
  });

  test('collects unknown symbols', () => {
    const rows = [
      { date: '2024-01-15', symbol: 'UNKNOWN', type: 'BUY', qty: '100', price: '150.00' },
    ];
    const report = validateCsvFile(rows, columnMapping, ['AAPL']);
    expect(report.unknownSymbols).toContain('UNKNOWN');
  });

  test('reports total rows and valid count', () => {
    const rows = [
      { date: '2024-01-15', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00' },
      { date: 'bad-date', symbol: 'AAPL', type: 'BUY', qty: '100', price: '150.00' },
    ];
    const report = validateCsvFile(rows, columnMapping, ['AAPL']);
    expect(report.totalRows).toBe(2);
    expect(report.validRows).toBe(1);
    expect(report.errorRows).toHaveLength(1);
  });

  test('constants provide values for unmapped fields across all rows', () => {
    const rows = [
      { date: '2024-01-15', qty: '100', price: '150.00' },
      { date: '2024-02-20', qty: '50', price: '300.00' },
    ];
    const partialMapping: ColumnMapping = { 0: 'date', 1: 'quantity', 2: 'price' };
    const report = validateCsvFile(rows, partialMapping, ['AAPL'], { symbol: 'AAPL', type: 'BUY' });
    expect(report.validRows).toBe(2);
    expect(report.valid).toBe(true);
  });
});
