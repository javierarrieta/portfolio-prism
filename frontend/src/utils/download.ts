/**
 * Hand generated text to the browser as a file download.
 *
 * There was no export precedent anywhere in this app before this helper, so this is the one
 * small place the next export is expected to copy rather than re-derive. Kept separate from
 * the CSV builders so those stay free of DOM access and unit-testable in plain Node.
 */
export const downloadCsv = (filename: string, csv: string): void => {
  const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' }));
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  // The click starts the download synchronously, so the URL is already spent by the time
  // this returns; revoking here is what keeps the Blob from leaking for the page's lifetime.
  URL.revokeObjectURL(url);
};
