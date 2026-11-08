#!/bin/bash

# Kill all background processes started by this script upon exit
trap "kill 0" EXIT

echo "================================================"
echo "🚀 Starting Portfolio Prism..."
echo "================================================"

# 1. Start Backend Rust
echo "⚙️  Starting Backend API on http://127.0.0.1:8000..."
cd backend_rust
# Development only. The release image compiles the `disabled` variant out, so this
# line is what lets a local run skip an identity provider — it is not a shortcut that
# also works in production.
#
# No ROCKET_SECRET_KEY here on purpose: `cargo run` is a debug build, which resolves
# to Rocket's `debug` profile, where Rocket generates a temporary key itself. The
# consequence is that dev sessions end whenever the backend restarts. Do not "fix"
# that by exporting a key from this script — a committed key is a leaked key.
export AUTH_MODE=disabled
cargo run &
BACKEND_PID=$!
cd ..

# Wait for backend to be ready
for i in $(seq 1 15); do
  if curl -s http://127.0.0.1:8000/ > /dev/null 2>&1; then
    echo "✅ Backend is running"
    break
  fi
  if ! kill -0 $BACKEND_PID 2>/dev/null; then
    echo "❌ Backend failed to start"
    exit 1
  fi
  sleep 0.5
done

if ! curl -s http://127.0.0.1:8000/ > /dev/null 2>&1; then
  echo "❌ Backend failed to become ready"
  exit 1
fi

# 2. Start Frontend Vite
echo "💻 Starting Frontend Web App on http://localhost:5173..."
cd frontend
npm run dev &
FRONTEND_PID=$!
cd ..

# Wait for frontend to be ready
for i in $(seq 1 15); do
  if curl -s http://localhost:5173/ > /dev/null 2>&1; then
    echo "✅ Frontend is running"
    break
  fi
  if ! kill -0 $FRONTEND_PID 2>/dev/null; then
    echo "❌ Frontend failed to start"
    exit 1
  fi
  sleep 0.5
done

if ! curl -s http://localhost:5173/ > /dev/null 2>&1; then
  echo "❌ Frontend failed to become ready"
  exit 1
fi

echo "================================================"
echo "✨ Both servers are running!"
echo "👉 Web App: http://localhost:5173"
echo "👉 API: http://127.0.0.1:8000"
echo "Press CTRL+C to stop both servers."
echo "================================================"

# Wait for background processes to finish (will block terminal)
wait
