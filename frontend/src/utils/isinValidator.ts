/**
 * Shape **and** the ISO 6166 check digit.
 *
 * This is deliberately the same rule as `is_valid_isin`
 * (`backend_rust/src/api_routes/lookup.rs`), which every write and the lookup
 * route enforce. The two agreeing is the point: this function gates the ISIN field
 * in `PortfolioDetail`, and the API answers a wrong digit with a bare 400 that the
 * UI can only report as "ISIN not found". A shape-only check here therefore did not
 * produce a gentler error than the backend, it produced a vaguer one — after a
 * round trip to a provider that cannot resolve the instrument either.
 *
 * The algorithm: expand letters to two digits each (`A`=10 … `Z`=35), Luhn over the
 * expanded body (the first eleven characters), compare with the twelfth.
 */
export type IsinProblem = 'shape' | 'checkDigit'

/**
 * Why this ISIN is not usable, or `null` when it is.
 *
 * A reason rather than a boolean because the two failures want different words:
 * a shape problem is "wrong length or characters", a check-digit problem means the
 * string is well-formed and one character is mistyped. Saying which saves the user
 * from re-typing an ISIN they copied correctly except for the last character.
 */
export function isinProblem(isin: string): IsinProblem | null {
  const upper = isin.toUpperCase()
  if (!/^[A-Z]{2}[A-Z0-9]{10}$/.test(upper)) return 'shape'
  const expected = isinCheckDigit(upper.slice(0, 11))
  // `Number('A')` is NaN, so a letter in the twelfth position fails the
  // comparison below rather than passing it — which is what the backend does too.
  return expected === Number(upper[11]) ? null : 'checkDigit'
}

export function isValidIsin(isin: string): boolean {
  return isinProblem(isin) === null
}

/**
 * The ISO 6166 check digit for the first eleven characters, or `null` if they are
 * not a valid body.
 *
 * Letters become two decimal digits each, so the expanded form is longer than the
 * input. Luhn then runs from the right of the body: the digit immediately left of
 * the check digit is doubled, then every second one after that.
 */
function isinCheckDigit(body: string): number | null {
  const digits: number[] = []
  for (const ch of body) {
    if (ch >= '0' && ch <= '9') {
      digits.push(Number(ch))
    } else if (ch >= 'A' && ch <= 'Z') {
      // 'A' is 65 and stands for 10.
      const value = ch.charCodeAt(0) - 65 + 10
      digits.push(Math.floor(value / 10), value % 10)
    } else {
      return null
    }
  }

  let sum = 0
  for (let i = 0; i < digits.length; i++) {
    let d = digits[digits.length - 1 - i]
    if (i % 2 === 0) {
      d *= 2
      if (d > 9) d -= 9
    }
    sum += d
  }
  return (10 - (sum % 10)) % 10
}
