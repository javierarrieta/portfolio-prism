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
  // An off-document anchor has not always been enough: Safari and older Firefox are on
  // record for dropping a download whose anchor was never in the document, so it goes in and
  // straight back out again.
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  // Deferred one task rather than done on the next line. Chrome has the transfer underway
  // by the time click() returns, but that is browser behaviour and not the spec, and
  // revoking in the same tick has been seen to cancel the download elsewhere. One task is
  // long enough for the handover and short enough that the Blob does not outlive the click.
  setTimeout(() => URL.revokeObjectURL(url), 0);
};
