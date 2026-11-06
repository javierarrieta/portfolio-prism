/**
 * Single source for the API prefix. Everything that talks to the backend has to
 * use this rather than retyping '/api': the Vite dev proxy and the production
 * nginx location both key off this exact string, so a component that hardcodes
 * it is one proxy-rule edit away from silently 404ing in one environment only.
 */
export const API_BASE = '/api';
