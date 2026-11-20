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
 * A decimal in the form the API accepts.
 *
 * Deliberately narrower than `parseFloat`. The backend's `strict_decimal_json`
 * refuses anything it cannot name, so a cell that parses here and is then refused
 * there costs the user a failed import row and a round trip. Exponent form is
 * excluded for that reason: `1e3` is a number to `parseFloat` and is not a
 * decimal string.
 */
const DECIMAL_ONLY = /^-?\d+(\.\d+)?$/;

/**
 * Parse one CSV cell into the exact decimal string that will be sent to the API,
 * or `null` if the cell is not a decimal.
 *
 * No binary float is involved, and that is the entire point. `parseFloat` is how
 * `9.99` became a value the tax engine could not trust; routing the write path
 * back through a float would reintroduce at the boundary exactly what the string
 * transport exists to prevent. The cell is cleaned, matched, and canonicalized as
 * text.
 *
 * Currency symbols, thousands separators and whitespace are stripped first, as
 * before. Note that this cannot tell a thousands separator from a decimal comma —
 * `1,50` becomes `150`. That ambiguity predates this function and is not solved
 * here; it needs a locale decision, not a regex.
 */
export function parseDecimal(val: string): string | null {
  const cleaned = (val ?? '').replace(/[$€£¥₹,\s]/g, '');
  if (!DECIMAL_ONLY.test(cleaned)) return null;

  const negative = cleaned.startsWith('-');
  const body = negative ? cleaned.slice(1) : cleaned;
  const [intPart, fracPart] = body.split('.');
  const int = intPart.replace(/^0+/, '') || '0';
  const frac = fracPart ? fracPart.replace(/0+$/, '') : '';
  const out = frac ? `${int}.${frac}` : int;
  // `-0` is the same number as `0` and would otherwise make two identical rows
  // look different to the import dedup key.
  return negative && out !== '0' ? `-${out}` : out;
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

  const val = parseDecimal(qtyStr);
  if (val === null) {
    errors.push({ row, field: 'quantity', message: `Invalid quantity: "${qtyStr}"` });
    return errors;
  }

  // `Number()` here is for the sign test only; the value that gets stored is the
  // exact string. Sign and zero survive the conversion, magnitude does not.
  if (Number(val) <= 0) {
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

  const val = parseDecimal(priceStr);
  if (val === null) {
    errors.push({ row, field: 'price', message: `Invalid price: "${priceStr}"` });
    return errors;
  }

  if (Number(val) < 0) {
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

  const val = parseDecimal(feeStr);
  if (val === null) {
    errors.push({ row, field: 'fee', message: `Invalid fee: "${feeStr}"` });
    return errors;
  }

  if (Number(val) < 0) {
    errors.push({ row, field: 'fee', message: `Fee must be non-negative, got ${val}` });
  }

  return errors;
}

export interface ValidatedRow {
  row: number;
  date: string;
  symbol: string;
  type: 'BUY' | 'SELL' | 'SPLIT';
  /**
   * Decimal strings, not numbers. These are posted to the API verbatim, and the
   * API takes money as an exact decimal string; a `number` here would mean the
   * value had already been through a binary float before it was ever sent.
   */
  quantity: string;
  price: string;
  fee: string;
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

  const quantity = parseDecimal(qtyRaw) ?? '0';
  const price = parseDecimal(priceRaw) ?? '0';

  if (normalizedType === 'SPLIT' && !(Number(price) > 0)) {
    allErrors.push({ row: rowNum, field: 'price', message: 'SPLIT ratio denominator (shares before) must be positive' });
  }

  return {
    row: rowNum,
    date: dateRaw,
    symbol: symbolRaw.trim(),
    type: normalizedType,
    quantity,
    price,
    fee: normalizedType === 'SPLIT' ? '0' : (parseDecimal(feeRaw ?? '0') ?? '0'),
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
