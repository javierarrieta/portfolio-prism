import { http, HttpResponse } from 'msw'
import type { Portfolio, Asset } from '../../types'
import type { AssetType } from '../../types'

export const handlers = [
  http.get('/api/portfolios', () => {
    const mockPortfolios: Portfolio[] = [
      {
        id: 1,
        name: 'Test Portfolio',
        description: 'A test portfolio',
        currency: 'USD',
        assets: [
          {
            id: 1,
            portfolio_id: 1,
            symbol: 'AAPL',
            name: 'Apple Inc.',
            asset_type: 'STOCK',
            sector: 'Technology',
            currency: 'USD',
            transactions: []
          }
        ]
      },
      {
        id: 2,
        name: 'Retirement Fund',
        description: 'Retirement savings',
        currency: 'USD',
        assets: []
      }
    ]
    return HttpResponse.json(mockPortfolios)
  }),
  http.get('/api/portfolios/:id', ({ params }) => {
    const id = Number(params.id)
    const mockPortfolio: Portfolio = {
      id,
      name: `Portfolio ${id}`,
      description: `Description for portfolio ${id}`,
      currency: 'USD',
      assets: [
        {
          id: 1,
          portfolio_id: id,
          symbol: 'AAPL',
          name: 'Apple Inc.',
          asset_type: 'STOCK',
          sector: 'Technology',
          currency: 'USD',
          transactions: []
        }
      ]
    }
    return HttpResponse.json(mockPortfolio)
  }),
  http.post('/api/portfolios', async ({ request }) => {
    const body = (await request.json()) as Partial<Portfolio>
    const newPortfolio: Portfolio = {
      id: 3,
      name: body.name || 'New Portfolio',
      description: body.description,
      currency: body.currency || 'USD',
      assets: []
    }
    return HttpResponse.json(newPortfolio, { status: 201 })
  }),
  http.get('/api/assets/:id', ({ params }) => {
    const id = Number(params.id)
    const mockAsset: Asset = {
      id,
      portfolio_id: 1,
      symbol: 'AAPL',
      name: 'Apple Inc.',
      asset_type: 'STOCK',
      sector: 'Technology',
      currency: 'USD',
      transactions: []
    }
    return HttpResponse.json(mockAsset)
  }),
  http.post('/api/portfolios/:portfolioId/assets', async ({ params, request }) => {
    const { portfolioId } = params
    const body = (await request.json()) as { symbol: string; name: string; asset_type: string; currency: string }
    const newAsset: Asset = {
      id: 999,
      portfolio_id: Number(portfolioId),
      symbol: body.symbol.toUpperCase(),
      name: body.name,
      asset_type: body.asset_type.toUpperCase() as AssetType,
      sector: undefined,
      currency: body.currency || 'USD',
      transactions: []
    }
    return HttpResponse.json(newAsset, { status: 201 })
  }),
  http.post('/api/portfolios/:portfolioId/assets/:assetId/transactions', async ({ params, request }) => {
    const { assetId } = params
    const body = (await request.json()) as { type: string; quantity: number; price: number; fee: number; date: string }
    return HttpResponse.json({
      id: 1000,
      asset_id: Number(assetId),
      type: body.type.toUpperCase(),
      quantity: String(body.quantity),
      price: String(body.price),
      fee: String(body.fee),
      date: body.date
    }, { status: 201 })
  }),
   http.delete('/api/portfolios/:id', () => {
     return HttpResponse.json(null, { status: 204 })
   }),
   http.get('/api/portfolios/:id/performance', () => {
     return HttpResponse.json({ metrics: { portfolio_value: 5000 } })
   }),
   http.get('/api/portfolios/:id/tax-summary', () => {
     return HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })
   }),
   http.get('/api/portfolios/:id/transactions', () => {
     return HttpResponse.json([
       { id: 1, asset_id: 1, type: 'BUY', quantity: 100, price: 50, fee: 0, date: '2024-01-15T00:00:00Z' },
       { id: 2, asset_id: 1, type: 'SPLIT', quantity: 2, price: 1, fee: 0, date: '2024-06-01T00:00:00Z' },
     ])
   }),
   // Registered before the :id route on purpose: MSW takes the first match, so a
   // wildcard below would make any specific route above it unreachable.
   http.get('/api/transactions/999/pnl', () => new HttpResponse(null, { status: 404 })),
   http.get('/api/transactions/:id/pnl', ({ params }) => {
     const id = Number(params.id)
     return HttpResponse.json({
       transaction_id: id, asset_id: 1, symbol: 'AAPL', asset_type: 'STOCK',
       strategy: 'FIFO', threshold_days: 30,
       currency: 'USD', asset_currency: 'USD', fx_applied: false, split_applied: false,
       sell_date: '2024-12-01T00:00:00Z',
       sold_qty: '80.0', price: '130.0', fee: '0.0',
       proceeds: '10400.0', cost_basis: '8400.0', realized_pnl: '2000.0', roi: '0.238',
       matched_qty: '80.0', unmatched_qty: '0.0',
       matches: [
         { buy_transaction_id: 1, buy_date: '2024-01-01T00:00:00Z', buy_price: '100.0',
           unit_cost: '100.0', matched_qty: '60.0', lot_qty_before_match: '60.0',
           cost_basis: '6000.0', proceeds: '7800.0', pnl: '1800.0', holding_days: 335, term: null },
         { buy_transaction_id: 2, buy_date: '2024-06-01T00:00:00Z', buy_price: '120.0',
           unit_cost: '120.0', matched_qty: '20.0', lot_qty_before_match: '40.0',
           cost_basis: '2400.0', proceeds: '2600.0', pnl: '200.0', holding_days: 183, term: null },
       ],
     })
   }),
   http.all('*', () => {
     return HttpResponse.json({ error: 'not implemented' }, { status: 501 })
   })
 ]
