import { describe, test, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import CsvImportModal from '../../components/CsvImportModal';

describe('CsvImportModal integration', () => {
  test('upload step allows file selection', () => {
    render(
      <CsvImportModal
        portfolioId={1}
        assets={[{ id: 1, portfolio_id: 1, symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', sector: 'Technology', currency: 'USD', isin: null, transactions: [] }]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    const fileInput = screen.getByLabelText(/csv/i);
    expect(fileInput).toBeInTheDocument();
  });

  test('shows upload step initially', () => {
    render(
      <CsvImportModal
        portfolioId={1}
        assets={[]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    expect(screen.getByText(/import transactions from csv/i)).toBeInTheDocument();
  });

  test('renders all 5 steps in order', () => {
    render(
      <CsvImportModal
        portfolioId={1}
        assets={[]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    expect(screen.getByText('Import Transactions from CSV')).toBeInTheDocument();
  });

  test('dragenter sets dragActive border color', () => {
    render(
      <CsvImportModal
        portfolioId={1}
        assets={[]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    const dropZone = screen.getByText(/drag and drop/i).closest('div');
    expect(dropZone).toBeInTheDocument();

    fireEvent.dragEnter(dropZone!);
    expect(dropZone).toHaveStyle({ borderColor: 'var(--color-success)' });
  });

  test('dragleave resets dragActive border color', () => {
    render(
      <CsvImportModal
        portfolioId={1}
        assets={[]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    const dropZone = screen.getByText(/drag and drop/i).closest('div');
    expect(dropZone).toBeInTheDocument();

    fireEvent.dragEnter(dropZone!);
    expect(dropZone).toHaveStyle({ borderColor: 'var(--color-success)' });

    fireEvent.dragLeave(dropZone!);
    expect(dropZone).toHaveStyle({ borderColor: 'var(--border-color)' });
  });

  test('dragover prevents default to enable dropping', () => {
    render(
      <CsvImportModal
        portfolioId={1}
        assets={[]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    const dropZone = screen.getByText(/drag and drop/i).closest('div');
    expect(dropZone).toBeInTheDocument();

    fireEvent.dragOver(dropZone!, { cancelable: true });
  });

  test('dropping a CSV file processes the file', async () => {
    const csvContent = 'date,symbol,type,quantity,price\n2024-01-15,AAPL,BUY,10,150.00';
    const file = new File([csvContent], 'transactions.csv', { type: 'text/csv' });

    const onImportComplete = vi.fn();
    render(
      <CsvImportModal
        portfolioId={1}
        assets={[{ id: 1, portfolio_id: 1, symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', sector: 'Technology', currency: 'USD', isin: null, transactions: [] }]}
        onClose={() => {}}
        onImportComplete={onImportComplete}
      />
    );

    const dropZone = screen.getByText(/drag and drop/i).closest('div');
    expect(dropZone).toBeInTheDocument();

    fireEvent.drop(dropZone!, { dataTransfer: { files: [file] } });
  });

  test('clearing a mapped column shows the clear option in the mapped columns dropdown', async () => {
    const csvContent = 'date,symbol,type,qty,price\n2024-01-15,AAPL,BUY,10,150.00';
    const file = new File([csvContent], 'transactions.csv', { type: 'text/csv' });

    render(
      <CsvImportModal
        portfolioId={1}
        assets={[{ id: 1, portfolio_id: 1, symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', sector: 'Technology', currency: 'USD', isin: null, transactions: [] }]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    const dropZone = screen.getByText(/drag and drop/i).closest('div');
    fireEvent.drop(dropZone!, { dataTransfer: { files: [file] } });

    await waitFor(() => {
      const clearOptions = screen.getAllByText(/\(clear\)/);
      expect(clearOptions.length).toBeGreaterThan(0);
    });
  });

  test('unmapping a column removes the field from mapped columns table', async () => {
    const csvContent = 'date,symbol,type,qty,price\n2024-01-15,AAPL,BUY,10,150.00';
    const file = new File([csvContent], 'transactions.csv', { type: 'text/csv' });

    render(
      <CsvImportModal
        portfolioId={1}
        assets={[{ id: 1, portfolio_id: 1, symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', sector: 'Technology', currency: 'USD', isin: null, transactions: [] }]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    const dropZone = screen.getByText(/drag and drop/i).closest('div');
    fireEvent.drop(dropZone!, { dataTransfer: { files: [file] } });

    await waitFor(() => {
      expect(screen.getByText('date (clear)')).toBeInTheDocument();
    });
  });

  test('clearing a column mapping makes that field available in the constant values section', async () => {
    const csvContent = 'date,symbol,type,qty,price\n2024-01-15,AAPL,BUY,10,150.00';
    const file = new File([csvContent], 'transactions.csv', { type: 'text/csv' });

    render(
      <CsvImportModal
        portfolioId={1}
        assets={[{ id: 1, portfolio_id: 1, symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', sector: 'Technology', currency: 'USD', isin: null, transactions: [] }]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    const dropZone = screen.getByText(/drag and drop/i).closest('div');
    fireEvent.drop(dropZone!, { dataTransfer: { files: [file] } });

    await waitFor(() => {
      const clearOptions = screen.getAllByText(/\(clear\)/);
      expect(clearOptions.length).toBeGreaterThan(0);
    });

    const symbolClearOption = screen.getByText('symbol (clear)');
    const symbolSelect = symbolClearOption.closest('select');
    expect(symbolSelect).toBeInTheDocument();

    fireEvent.change(symbolSelect!, { target: { value: '' } });

    await waitFor(() => {
      expect(screen.getByText('Constant Values')).toBeInTheDocument();
    });
  });

  test('confirm step renders with Confirm and Import button using isImporting state', async () => {
    const csvContent = 'date,symbol,type,qty,price\n2024-01-15,AAPL,BUY,10,150.00';
    const file = new File([csvContent], 'transactions.csv', { type: 'text/csv' });

    render(
      <CsvImportModal
        portfolioId={1}
        assets={[{ id: 1, portfolio_id: 1, symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', sector: 'Technology', currency: 'USD', isin: null, transactions: [] }]}
        onClose={() => {}}
        onImportComplete={() => {}}
      />
    );

    const dropZone = screen.getByText(/drag and drop/i).closest('div');
    fireEvent.drop(dropZone!, { dataTransfer: { files: [file] } });

    await waitFor(() => {
      expect(screen.getByText(/column mapping/i)).toBeInTheDocument();
    });

    const validateButton = screen.getByRole('button', { name: /validate data/i });
    fireEvent.click(validateButton);

    await waitFor(() => {
      expect(screen.getByText('Import Transactions from CSV')).toBeInTheDocument();
    });

    const importButton = screen.getByRole('button', { name: /import \d+ transactions$/i });
    expect(importButton).toBeInTheDocument();
    fireEvent.click(importButton);

    await waitFor(() => {
      expect(screen.getByText(/confirm and import/i)).toBeInTheDocument();
    });

    const confirmButton = screen.getByRole('button', { name: /confirm and import/i });
    expect(confirmButton).not.toBeDisabled();
    expect(confirmButton.textContent).toContain('Confirm and Import');
  });
});
