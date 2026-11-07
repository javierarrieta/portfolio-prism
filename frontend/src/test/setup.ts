// Host-timezone dependent toLocaleDateString/toLocaleString calls in the
// sell P&L tests would fail on any machine west of UTC.
process.env.TZ = 'UTC'
import '@testing-library/jest-dom/vitest'
import { afterAll, afterEach, beforeAll } from 'vitest'
import { server } from './mocks/server'

beforeAll(() => server.listen({ onUnhandledRequest: 'bypass' }))
afterEach(() => server.resetHandlers())
afterAll(() => server.close())
