const DEFAULT_STORAGE_KEY = 'hone-start-state';

function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, (ch) => {
    switch (ch) {
      case '&': return '&amp;';
      case '<': return '&lt;';
      case '>': return '&gt;';
      case '"': return '&quot;';
      case "'": return '&#39;';
      default: return ch;
    }
  });
}

function loadStoredState(storageKey = DEFAULT_STORAGE_KEY) {
  if (typeof localStorage === 'undefined') {
    return { current: 0, completed: [] };
  }
  try {
    const parsed = JSON.parse(localStorage.getItem(storageKey) || '{}');
    if (parsed && typeof parsed === 'object') {
      return {
        current: typeof parsed.current === 'number' && Number.isFinite(parsed.current) ? parsed.current : 0,
        completed: Array.isArray(parsed.completed) ? parsed.completed.map(Boolean) : [],
      };
    }
  } catch (_) {}
  return { current: 0, completed: [] };
}

function normalizeSteps(manifest) {
  return Array.isArray(manifest?.ordered_steps) ? manifest.ordered_steps : [];
}

async function fetchStartManifest(url = '/start.json') {
  const response = await fetch(url, { cache: 'no-store' });
  if (!response.ok) {
    throw new Error(`Unable to load ${url}: ${response.status}`);
  }
  return response.json();
}

function pickCurrentStep(manifest, storageKey = DEFAULT_STORAGE_KEY) {
  const steps = normalizeSteps(manifest);
  const stored = loadStoredState(storageKey);
  if (!steps.length) {
    return { steps, currentIndex: 0, currentStep: null, stored };
  }
  let index = stored.current;
  if (!Number.isFinite(index) || index < 0 || index >= steps.length) {
    index = 0;
  }
  if (stored.completed[index]) {
    for (let i = 0; i < steps.length; i += 1) {
      if (!stored.completed[i]) {
        index = i;
        break;
      }
    }
  }
  return { steps, currentIndex: index, currentStep: steps[index] || steps[0] || null, stored };
}

function doneWhenHtml(step) {
  return (step?.done_when || [])
    .map((item) => `<li>${escapeHtml(item)}</li>`)
    .join('');
}

function renderStepSummary(step) {
  if (!step) {
    return {
      title: 'HONE Start',
      instruction: 'Loading the HONE start manifest…',
      doneWhen: '',
      currentMarkup: '<strong>Loading manifest…</strong><div class="start-rail-meta">The next HONE step will appear here once /start.json loads.</div>',
    };
  }
  return {
    title: step.title || 'HONE Start',
    instruction: step.instruction || 'Continue the HONE start wizard.',
    doneWhen: doneWhenHtml(step),
    currentMarkup: `
      <strong>Current HONE step: ${escapeHtml(step.title || 'HONE Start')}</strong>
      <div class="start-rail-meta">${escapeHtml(step.instruction || 'Continue the HONE start wizard.')}</div>
      <strong style="margin-top:8px;display:block;">Done when:</strong>
      <ul>${doneWhenHtml(step)}</ul>
    `.trim(),
  };
}

function hydrateBanner({ manifest, bannerSelector, titleSelector, metaSelector, currentSelector, storageKey = DEFAULT_STORAGE_KEY }) {
  const { currentStep } = pickCurrentStep(manifest, storageKey);
  const step = currentStep;
  const banner = typeof bannerSelector === 'string' ? document.querySelector(bannerSelector) : bannerSelector;
  const title = typeof titleSelector === 'string' ? document.querySelector(titleSelector) : titleSelector;
  const meta = typeof metaSelector === 'string' ? document.querySelector(metaSelector) : metaSelector;
  const current = typeof currentSelector === 'string' ? document.querySelector(currentSelector) : currentSelector;
  const summary = renderStepSummary(step);
  if (title) title.textContent = summary.title;
  if (meta) meta.textContent = `${summary.instruction} The manifest stays synchronized for agents.`;
  if (current) current.innerHTML = summary.currentMarkup;
  if (banner && !step) {
    banner.setAttribute('data-start-state', 'loading');
  }
  return { step, summary };
}

function renderStepRail({ container, steps, currentIndex, storageKey = DEFAULT_STORAGE_KEY, onSelect }) {
  const el = typeof container === 'string' ? document.querySelector(container) : container;
  if (!el) return;
  el.innerHTML = steps
    .map((step, idx) => {
      const classes = [];
      if (idx === currentIndex) classes.push('current');
      else if (idx < currentIndex) classes.push('done');
      return `
        <button type="button" class="${classes.join(' ')}" data-step-index="${idx}">
          <span class="start-order-title">${idx + 1}. ${escapeHtml(step.title || 'Start step')}</span>
          <span class="start-order-meta">${escapeHtml(step.instruction || step.why || 'Continue the HONE start wizard.')}</span>
        </button>
      `.trim();
    })
    .join('');
  el.querySelectorAll('button[data-step-index]').forEach((btn) => {
    btn.addEventListener('click', () => {
      const idx = Number(btn.getAttribute('data-step-index'));
      if (Number.isFinite(idx) && typeof onSelect === 'function') onSelect(idx);
    });
  });
}

function attachStartStateApi(target = window) {
  target.HoneStartState = {
    DEFAULT_STORAGE_KEY,
    escapeHtml,
    fetchStartManifest,
    loadStoredState,
    normalizeSteps,
    pickCurrentStep,
    renderStepSummary,
    hydrateBanner,
    renderStepRail,
  };
}

attachStartStateApi();
