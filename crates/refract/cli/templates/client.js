(() => {
  const script = document.currentScript;
  const session = script.dataset.session;
  const revision = Number(script.dataset.revision);
  let overlay;
  function showError(message) {
    if (!overlay) {
      overlay = document.createElement('div');
      overlay.setAttribute('role', 'alert');
      Object.assign(overlay.style, {
        position: 'fixed', inset: '0', zIndex: '2147483647', padding: '32px',
        background: '#18181bf5', color: '#fafafa', overflow: 'auto',
        font: '14px/1.5 ui-monospace, monospace', whiteSpace: 'pre-wrap',
      });
      document.body.append(overlay);
    }
    overlay.textContent = `Refract could not rebuild\n\n${message}\n\nSave a file to retry. Your last successful build is still available.`;
  }
  async function poll() {
    try {
      const response = await fetch('/__refract/status', { cache: 'no-store' });
      if (!response.ok) throw new Error('Development server unavailable');
      const state = await response.json();
      if (state.session !== session || state.revision !== revision) {
        location.reload();
        return;
      }
      if (state.error) showError(state.error);
      else if (overlay) { overlay.remove(); overlay = undefined; }
    } catch {
      // Keep the app usable when the dev server is stopped; reconnect on restart.
    }
    setTimeout(poll, 500);
  }
  poll();
})();
