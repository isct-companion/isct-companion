(request => {
  if (request.cancelRequest) {
    // A cancel can reach the UI thread before the upload evaluation.
    (globalThis.__isctCancelledAssignmentUploads ??= new Set()).add(request.cancelRequest);
    const upload = globalThis.__isctAssignmentUpload;
    if (upload?.request === request.cancelRequest) upload.cancel(true);
    return;
  }
  if (globalThis.__isctCancelledAssignmentUploads?.has(request.request)) return;
  const target = new URL(request.url);
  const matches = () => {
    const here = new URL(location.href);
    return here.searchParams.getAll('id').length === 1 && here.searchParams.getAll('action').length === 1 &&
      here.origin === target.origin && here.pathname === target.pathname &&
      here.searchParams.get('id') === target.searchParams.get('id') &&
      here.searchParams.get('action') === 'editsubmission' &&
      (!request.scoped || (here.searchParams.getAll('isctDraftRequest').length === 1 &&
        here.searchParams.get('isctDraftRequest') === request.request));
  };
  if (!matches()) return;
  if (globalThis.__isctAssignmentUpload?.request === request.request) return;
  globalThis.__isctAssignmentUpload?.cancel();
  let observer, timer;
  const cancel = (removeStatus = false) => {
    observer?.disconnect();
    clearTimeout(timer);
    removeEventListener('pagehide', cancelOnHide);
    request.files = [];
    if (removeStatus) banner.remove();
  };
  const cancelOnHide = () => cancel();
  globalThis.__isctAssignmentUpload = { request: request.request, cancel };
  addEventListener('pagehide', cancelOnHide, { once: true });
  const banner = document.createElement('p');
  banner.setAttribute('role', 'status');
  banner.id = 'isct-assignment-upload-status';
  banner.textContent = request.messages.waiting;
  const field = () => document.querySelector('input[name="files_filemanager"]');
  const form = field()?.form;
  document.getElementById(banner.id)?.remove();
  (form || document.body).prepend(banner);
  const fail = () => { banner.textContent = request.messages.failed; cancel(); };
  const upload = () => {
    if (!request.files.length || !matches()) return cancel();
    const input = field();
    const area = input?.closest('.fitem')?.querySelector('.filemanager.fm-loaded');
    if (!area || area.classList.contains('fm-updating')) return;
    try {
      if (!input.form || input.disabled || input.closest('.disabled')) return fail();
      // Reuse Moodle's own drag-and-drop uploader, including its draft-area
      // validation and progress UI. Do not reproduce its authenticated API.
      const transfer = new DataTransfer();
      for (const file of request.files) {
        const bytes = Uint8Array.from(atob(file.data), character => character.charCodeAt(0));
        transfer.items.add(new File([bytes], file.name, { type: 'application/octet-stream' }));
      }
      const dropzone = area.querySelector('.filemanager-container') || area;
      for (const type of ['dragenter', 'dragover']) {
        dropzone.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: transfer }));
      }
      const accepted = !dropzone.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: transfer }));
      banner.textContent = accepted ? request.messages.queued : request.messages.failed;
      cancel();
    } catch { fail(); }
  };
  observer = new MutationObserver(upload);
  observer.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['class'] });
  timer = setTimeout(fail, 15000);
  upload();
})
