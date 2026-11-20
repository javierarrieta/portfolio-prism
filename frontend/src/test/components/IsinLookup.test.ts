import { describe, it, expect } from 'vitest'
import { isinProblem, isValidIsin } from '../../utils/isinValidator'

describe('isValidIsin', () => {
  it('returns true for valid ISINs', () => {
    expect(isValidIsin('US0378331005')).toBe(true)
    expect(isValidIsin('DE000BAY0017')).toBe(true)
    expect(isValidIsin('us0378331005')).toBe(true)
  })

  it('returns false for ISINs that are too short', () => {
    expect(isValidIsin('US037833100')).toBe(false)
  })

  it('returns false for ISINs that are too long', () => {
    expect(isValidIsin('US03783310050')).toBe(false)
  })

  it('returns false for ISINs with non-alphanumeric characters', () => {
    expect(isValidIsin('US037833100!')).toBe(false)
  })

  it('returns false for ISINs without a country code', () => {
    expect(isValidIsin('0378331005')).toBe(false)
  })

  it('returns false for empty strings', () => {
    expect(isValidIsin('')).toBe(false)
  })

  it('returns false when only the check digit is wrong', () => {
    // The same vectors as `rejects_a_wrong_check_digit` in lookup.rs. The shape is
    // perfect in every one of these; before the digit was computed they all passed
    // here while the API refused them.
    expect(isValidIsin('US0378331004')).toBe(false)
    expect(isValidIsin('US0378331006')).toBe(false)
    expect(isValidIsin('IE00B3RBWM24')).toBe(false)
    expect(isValidIsin('BTC123456789')).toBe(false)
    // A letter in the twelfth position is shape-valid and never a digit.
    expect(isValidIsin('US037833100A')).toBe(false)
  })

  it('accepts the same live instruments the backend accepts', () => {
    // The ten vectors from `accepts_real_instruments`, including the three whose
    // bodies carry letters. If this list and the Rust one ever disagree, one of them
    // has drifted from ISO 6166.
    for (const isin of [
      'US0378331005',
      'US5949181045',
      'US88160R1014',
      'DE000BAY0017',
      'DE0007164600',
      'FR0000131906',
      'GB0002634946',
      'IE00B3RBWM25',
      'CH0038863350',
      'JP3756600007',
    ]) {
      expect(isValidIsin(isin), isin).toBe(true)
    }
  })

  it('names which kind of failure it found', () => {
    // The form words these differently: a shape problem is "wrong length or
    // characters", a check-digit problem is "well-formed, one character mistyped".
    expect(isinProblem('US037833100')).toBe('shape')
    expect(isinProblem('US037833100!')).toBe('shape')
    expect(isinProblem('0378331005')).toBe('shape')
    expect(isinProblem('US0378331004')).toBe('checkDigit')
    expect(isinProblem('US0378331005')).toBe(null)
    expect(isinProblem('us0378331005')).toBe(null)
  })
})