const TOKEN_STORAGE_KEY = 'fh-token';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

function loadToken(urlToken) {
  if (urlToken) {
    storeToken(urlToken);
    return urlToken;
  }
  try {
    return localStorage.getItem(TOKEN_STORAGE_KEY) || '';
  } catch {
    return '';
  }
}

function storeToken(token) {
  try {
    localStorage.setItem(TOKEN_STORAGE_KEY, token);
  } catch {
    // Storage may be blocked by the browser.
  }
}

export function createApiClient(urlToken, onUnauthorized) {
  let token = loadToken(urlToken);

  function setToken(nextToken) {
    token = nextToken;
    storeToken(token);
  }

  async function api(path, method = 'GET', body) {
    const options = { method, headers: { 'x-token': token } };
    if (body !== undefined) {
      options.headers['content-type'] = 'application/json';
      options.body = JSON.stringify(body);
    }
    const response = await fetch('/api' + path, options);
    const text = await response.text();
    if (response.status === 401) onUnauthorized();
    if (!response.ok) throw new Error(text || 'Ошибка ' + response.status);
    return text ? JSON.parse(text) : null;
  }

  function artUrl(gameId, kind) {
    return `/api/art/${encodeURIComponent(gameId)}/${kind}?t=${encodeURIComponent(token)}`;
  }

  function qrUrl(url) {
    return `/api/qr?t=${encodeURIComponent(token)}&v=${encodeURIComponent(url)}`;
  }

  /** Starts a server job and waits for it, reporting progress. */
  async function runJob(start, onTick) {
    const { job } = await start;
    for (;;) {
      await sleep(170);
      let status;
      try {
        status = await api('/job');
      } catch {
        continue;
      }
      if (!status || status.id !== job) continue;
      if (onTick) onTick(status);
      if (status.done) {
        if (status.error) throw new Error(status.error);
        return status.result;
      }
    }
  }

  return { api, artUrl, qrUrl, runJob, setToken };
}
