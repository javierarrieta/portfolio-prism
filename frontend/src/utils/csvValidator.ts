import type { ColumnMapping, CsvValidationRow } from '../types/csv';

interface DateFormat {
  regex: RegExp;
  parse: (s: string) => Date;
}

const DATE_FORMATS: DateFormat[] = [
  { regex: /^\d{4}-\d{2}-\d{2}$/, parse: (s) => new Date(s + 'T00:00:00Z') },
  { regex: /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/, parse: (s) => new Date(s.replace(' ', 'T') + ':00Z') },
  { regex: /^\d{2}\/\d{2}\/\d{4}$/, parse: (s) => {
    const parts = s.split('/');
    const first = parseInt(parts[0], 10);
    if (first > 12) {
      return new Date(`${parts[2]}-${parts[1]}-${parts[0]}T00:00:00Z`);
    }
    return new Date(`${parts[2]}-${parts[0]}-${parts[1]}T00:00:00Z`);
  }},
  { regex: /^\d{2}\/\d{2}\/\d{4} \d{2}:\d{2}$/, parse: (s) => {
    const [datePart, timePart] = s.split(' ');
    const parts = datePart.split('/');
    const first = parseInt(parts[0], 10);
    if (first > 12) {
      return new Date(`${parts[2]}-${parts[1]}-${parts[0]}T${timePart}:00Z`);
    }
    return new Date(`${parts[2]}-${parts[0]}-${parts[1]}T${timePart}:00Z`);
  }},
];

function isValidDate(dateStr: string): boolean {
  const trimmed = dateStr.trim();
  if (!trimmed) return false;
  return DATE_FORMATS.some((fmt) => fmt.regex.test(trimmed) && !isNaN(fmt.parse(trimmed).getTime()));
}

function parseDate(dateStr: string): Date | null {
  const trimmed = dateStr.trim();
  for (const fmt of DATE_FORMATS) {
    if (fmt.regex.test(trimmed)) {
      const d = fmt.parse(trimmed);
      if (!isNaN(d.getTime())) return d;
    }
  }
  return null;
}

function validateDate(
  dateStr: string,
  row: number
): CsvValidationRow[] {
  const errors: CsvValidationRow[] = [];

  if (!dateStr || !dateStr.trim()) {
    errors.push({ row, field: 'date', message: 'Date is required' });
    return errors;
  }

  if (!isValidDate(dateStr)) {
    errors.push({ row, field: 'date', message: `Invalid date format: "${dateStr}"` });
    return errors;
  }

  const parsed = parseDate(dateStr);
  if (parsed) {
    const year = parsed.getUTCFullYear();
    if (year < 1900 || year > 2100) {
      errors.push({ row, field: 'date', message: `Date year ${year} out of range (1900-2100)` });
    }
  }

  return errors;
}

function validateSymbol(
  symbol: string,
  row: number
): CsvValidationRow[] {
  const errors: CsvValidationRow[] = [];

  if (!symbol || !symbol.trim()) {
    errors.push({ row, field: 'symbol', message: 'Symbol is required' });
  }

  return errors;
}

function validateType(
  typeStr: string,
  row: number
): CsvValidationRow[] {
  const errors: CsvValidationRow[] = [];

  if (!typeStr || !typeStr.trim()) {
    errors.push({ row, field: 'type', message: 'Type is required' });
    return errors;
  }

  const upper = typeStr.trim().toUpperCase();
  if (upper !== 'BUY' && upper !== 'SELL' && upper !== 'SPLIT') {
    errors.push({ row, field: 'type', message: `Invalid type "${typeStr}", must be BUY, SELL or SPLIT` });
  }

  return errors;
}

/**
 * Fold thousands separators and decimal commas into a plain dot-decimal, or return
 * `null` when the cell cannot be read either way.
 *
 * The old code stripped every comma. That is not a simplification, it is a
 * corruption: `1,50` — one and a half, in the convention most European broker
 * exports use — became `150`. A hundredfold error on a cost-basis figure, applied
 * silently, on the row the tax engine will later trust. `1.234,56` became `123456`.
 *
 * Position decides this, not locale:
 *
 * - A comma with exactly three digits after it, and nothing after those, is a
 *   thousands separator: `1,500` → `1500`.
 * - A comma with one, two, or four-or-more digits after it cannot be a group of
 *   three, so it is a decimal comma: `1,50` → `1.5`, `1,5000` → `1.5`.
 * - When both `.` and `,` appear, the **later** one is the decimal separator and
 *   everything before it must group in threes: `1.234,56` → `1234.56`,
 *   `1,500.25` → `1500.25`.
 *
 * `1,500` is the one shape where both readings are legal. It keeps the reading it
 * has always had, because that is the status quo and the overwhelmingly common
 * intent — and because changing it silently would be the same mistake this fixes.
 *
 * What is illegal under both conventions is refused rather than guessed:
 * `12,34,567`, `1,5.25`, `1234,567` and `,5` all return `null`. Refusing is the
 * right default for money: a refused row is shown with a message naming it, while
 * a misread row is shown as a plausible, wrong portfolio, and nothing downstream
 * can tell the difference.
 */
function resolveSeparators(value: string): string | null {
  if (!value) return null;
  const negative = value.startsWith('-');
  const body = negative ? value.slice(1) : value;
  if (!body) return null;

  let out: string;
  if (!body.includes(',')) {
    // No comma: leave it alone and let parseFloat do what it has always done with it.
    out = body;
  } else {
    const lastDot = body.lastIndexOf('.');
    const lastComma = body.lastIndexOf(',');

    // Where is the decimal separator? With both present it is whichever comes
    // later. With commas only, a three-digit tail means there is none at all.
    let decPos: number;
    if (lastDot >= 0) {
      decPos = Math.max(lastDot, lastComma);
    } else {
      decPos = /^\d{3}$/.test(body.slice(lastComma + 1)) ? -1 : lastComma;
    }

    if (decPos === -1) {
      // Every comma is a thousands separator, so every group after the first must
      // be three digits. `12,34,567` fails here instead of becoming `1234567`.
      if (!/^\d{1,3}(,\d{3})*$/.test(body)) return null;
      out = body.replace(/,/g, '');
    } else {
      const tail = body.slice(decPos + 1);
      const head = body.slice(0, decPos);
      if (!/^\d+$/.test(tail)) return null;
      // The head is digits grouped by either separator, in threes.
      if (!/^\d{1,3}([.,]\d{3})*$/.test(head)) return null;
      out = head.replace(/[.,]/g, '') + '.' + tail;
    }
  }

  return negative ? `-${out}` : out;
}

function parseNumeric(val: string): number {
  // The comma is deliberately not in the strip class any more; `resolveSeparators`
  // needs to see it. Currency symbols and whitespace carry no ambiguity and still go.
  const cleaned = (val ?? '').replace(/[$€£¥₹\s]/g, '');
  const resolved = resolveSeparators(cleaned);
  // `NaN` is the callers' existing "not a number" signal — every caller tests it
  // with `isNaN`, so an unresolvable grouping reports as an invalid cell rather
  // than throwing its way through the validator.
  if (resolved === null) return NaN;
  return parseFloat(resolved);
}

function validateQuantity(
  qtyStr: string,
  row: number
): CsvValidationRow[] {
  const errors: CsvValidationRow[] = [];

  if (!qtyStr || !qtyStr.trim()) {
    errors.push({ row, field: 'quantity', message: 'Quantity is required' });
    return errors;
  }

  const val = parseNumeric(qtyStr);
  if (isNaN(val)) {
    errors.push({ row, field: 'quantity', message: `Invalid quantity: "${qtyStr}"` });
    return errors;
  }

  if (val <= 0) {
    errors.push({ row, field: 'quantity', message: `Quantity must be positive, got ${val}` });
  }

  return errors;
}

function validatePrice(
  priceStr: string,
  row: number
): CsvValidationRow[] {
  const errors: CsvValidationRow[] = [];

  if (!priceStr || !priceStr.trim()) {
    errors.push({ row, field: 'price', message: 'Price is required' });
    return errors;
  }

  const val = parseNumeric(priceStr);
  if (isNaN(val)) {
    errors.push({ row, field: 'price', message: `Invalid price: "${priceStr}"` });
    return errors;
  }

  if (val < 0) {
    errors.push({ row, field: 'price', message: `Price must be positive or zero, got ${val}` });
  }

  return errors;
}

function validateFee(
  feeStr: string | undefined,
  row: number
): CsvValidationRow[] {
  const errors: CsvValidationRow[] = [];

  if (!feeStr || !feeStr.trim()) {
    return errors;
  }

  const val = parseNumeric(feeStr);
  if (isNaN(val)) {
    errors.push({ row, field: 'fee', message: `Invalid fee: "${feeStr}"` });
    return errors;
  }

  if (val < 0) {
    errors.push({ row, field: 'fee', message: `Fee must be non-negative, got ${val}` });
  }

  return errors;
}

export interface ValidatedRow {
  row: number;
  date: string;
  symbol: string;
  type: 'BUY' | 'SELL' | 'SPLIT';
  quantity: number;
  price: number;
  fee: number;
  errors: CsvValidationRow[];
}

export interface ValidationReport {
  valid: boolean;
  totalRows: number;
  validRows: number;
  errorRows: CsvValidationRow[];
  unknownSymbols: string[];
  validatedRows: ValidatedRow[];
}

export function validateCsvRow(
  rowData: Record<string, string>,
  columnMapping: ColumnMapping,
  rowNum: number,
  constants: Record<string, string> = {}
): ValidatedRow {
  const getMapped = (field: string): string | undefined => {
    for (const [idx, mappedField] of Object.entries(columnMapping)) {
      if (mappedField === field) {
        const keys = Object.keys(rowData);
        const key = keys[parseInt(idx, 10)];
        if (key !== undefined) return rowData[key];
      }
    }
    return constants[field];
  };

  const dateRaw = getMapped('date') ?? '';
  const symbolRaw = getMapped('symbol') ?? '';
  const typeRaw = getMapped('type') ?? '';
  const qtyRaw = getMapped('quantity') ?? '';
  const priceRaw = getMapped('price') ?? '';
  const feeRaw = getMapped('fee');

  const allErrors: CsvValidationRow[] = [];

  allErrors.push(...validateDate(dateRaw, rowNum));
  allErrors.push(...validateSymbol(symbolRaw, rowNum));
  allErrors.push(...validateType(typeRaw, rowNum));
  allErrors.push(...validateQuantity(qtyRaw, rowNum));
  allErrors.push(...validatePrice(priceRaw, rowNum));
  allErrors.push(...validateFee(feeRaw, rowNum));

  const upperType = typeRaw.trim().toUpperCase();
  const normalizedType: 'BUY' | 'SELL' | 'SPLIT' = (upperType === 'SELL' ? 'SELL' : upperType === 'SPLIT' ? 'SPLIT' : 'BUY');

  if (normalizedType === 'SPLIT' && !(parseNumeric(priceRaw) > 0)) {
    allErrors.push({ row: rowNum, field: 'price', message: 'SPLIT ratio denominator (shares before) must be positive' });
  }

  return {
    row: rowNum,
    date: dateRaw,
    symbol: symbolRaw.trim(),
    type: normalizedType,
    quantity: parseNumeric(qtyRaw) || 0,
    price: parseNumeric(priceRaw) || 0,
    fee: normalizedType === 'SPLIT' ? 0 : (parseNumeric(feeRaw ?? '0') || 0),
    errors: allErrors,
  };
}

export function validateCsvFile(
  rows: Record<string, string>[],
  columnMapping: ColumnMapping,
  portfolioSymbols: string[],
  constants: Record<string, string> = {}
): ValidationReport {
  const allErrors: CsvValidationRow[] = [];
  const validatedRows: ValidatedRow[] = [];
  const unknownSymbols = new Set<string>();
  let validCount = 0;

  for (let i = 0; i < rows.length; i++) {
    const validated = validateCsvRow(rows[i], columnMapping, i + 1, constants);

    if (validated.errors.length > 0) {
      allErrors.push(...validated.errors);
    } else {
      validCount++;
    }

    const symbolUpper = validated.symbol.toUpperCase();
    if (symbolUpper && !portfolioSymbols.map((s) => s.toUpperCase()).includes(symbolUpper)) {
      unknownSymbols.add(symbolUpper);
    }

    validatedRows.push(validated);
  }

  return {
    valid: allErrors.length === 0,
    totalRows: rows.length,
    validRows: validCount,
    errorRows: allErrors,
    unknownSymbols: Array.from(unknownSymbols),
    validatedRows,
  };
}
