// Browser Redfish BMC client. Mirrors backend/src/integration/bmc/redfish.rs so
// the JS oob-agent (running in the operator's browser) executes the same
// endpoints/actions as the on-network Rust client. Browsers cannot skip TLS
// validation, so self-signed BMCs surface as a TypeError the caller can detect.

export interface BmcCreds {
  address: string;
  username: string;
  password: string;
  redfishPath: string;
  tlsInsecure: boolean;
  preferred: string;
  timeoutSecs: number;
  ipmiInterface: string;
}

export type PowerState = 'on' | 'off' | 'unknown';

interface Ctx {
  base: string;
  systemPath: string;
  username: string;
  password: string;
  timeoutMs: number;
}

function basicAuth(username: string, password: string): string {
  return 'Basic ' + btoa(username + ':' + password);
}

function systemUrl(ctx: Ctx): string {
  const p = ctx.systemPath.replace(/\/+$/, '');
  return p.startsWith('http') ? p : ctx.base + p;
}

async function rfFetch(ctx: Ctx, url: string, init: RequestInit = {}): Promise<Response> {
  const controller = new AbortController();
  const t = setTimeout(() => controller.abort(), ctx.timeoutMs);
  try {
    return await fetch(url, {
      ...init,
      signal: controller.signal,
      headers: {
        Accept: 'application/json',
        'User-Agent': 'tcs-bmc/0.3',
        Authorization: basicAuth(ctx.username, ctx.password),
        ...(init.headers || {}),
      },
    });
  } finally {
    clearTimeout(t);
  }
}

async function readJson(res: Response): Promise<any> {
  const text = await res.text();
  try {
    return JSON.parse(text);
  } catch {
    return {};
  }
}

async function requireOk(res: Response, what: string): Promise<void> {
  if (!res.ok && res.status !== 204 && res.status !== 202) {
    const text = await res.text().catch(() => '');
    throw new Error(`Redfish ${what} HTTP ${res.status}: ${text}`.trim());
  }
}

export async function connect(creds: BmcCreds): Promise<Ctx> {
  const host = creds.address
    .replace(/^https?:\/\//, '')
    .replace(/\/+$/, '');
  const base = `https://${host}`;
  const timeoutMs = Math.max(5, creds.timeoutSecs) * 1000;

  const res = await rfFetch(ctx0(base, creds.username, creds.password, timeoutMs), `${base}/redfish/v1/Systems`);
  if (!res.ok) throw new Error(`Redfish Systems HTTP ${res.status}`);
  const body = await readJson(res);

  let systemPath: string;
  const rp = (creds.redfishPath || '').trim();
  if (rp) {
    systemPath = rp.startsWith('/') ? rp : `/${rp}`;
  } else {
    const first = body?.Members?.[0]?.['@odata.id'];
    if (!first) throw new Error('Redfish: no Systems members found');
    systemPath = first;
  }

  return { base, systemPath, username: creds.username, password: creds.password, timeoutMs };
}

function ctx0(base: string, username: string, password: string, timeoutMs: number): Ctx {
  return { base, systemPath: '', username, password, timeoutMs };
}

export async function getPowerState(ctx: Ctx): Promise<PowerState> {
  const res = await rfFetch(ctx, systemUrl(ctx));
  await requireOk(res, 'get system');
  const body = await readJson(res);
  const state = body?.PowerState;
  if (state === 'On') return 'on';
  if (state === 'Off') return 'off';
  return 'unknown';
}

export async function power(ctx: Ctx, action: string): Promise<void> {
  let resetType: string;
  switch (action) {
    case 'on':
      resetType = 'On';
      break;
    case 'off':
      resetType = 'ForceOff';
      break;
    case 'reset':
    case 'cycle':
      resetType = 'ForceRestart';
      break;
    case 'graceful_shutdown':
      resetType = 'GracefulShutdown';
      break;
    default:
      throw new Error(`Unknown power action: ${action}`);
  }
  const res = await rfFetch(ctx, `${systemUrl(ctx)}/Actions/ComputerSystem.Reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ ResetType: resetType }),
  });
  await requireOk(res, 'power');
}

export async function setBoot(ctx: Ctx, target: 'pxe' | 'disk', once: boolean): Promise<void> {
  const body = {
    BootSourceOverrideTarget: target === 'pxe' ? 'Pxe' : 'Hdd',
    BootSourceOverrideEnabled: once ? 'Once' : 'Continuous',
    BootSourceOverrideMode: 'Legacy',
  };
  const res = await rfFetch(ctx, systemUrl(ctx), {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json', 'OData-Version': '4.0' },
    body: JSON.stringify(body),
  });
  await requireOk(res, 'set boot');
}

async function findMedia(ctx: Ctx, media: string): Promise<{ odata: string; names: string[] }> {
  const res = await rfFetch(ctx, `${systemUrl(ctx)}/VirtualMedia`);
  await requireOk(res, 'get VirtualMedia');
  const body = await readJson(res);
  const members: any[] = body?.Members || [];
  if (members.length === 0) throw new Error('No VirtualMedia members found');
  const names = members.map((m) => m?.Name || '');
  const match = members.find((m) => (m?.Name || '').toLowerCase() === media.toLowerCase());
  if (!match) {
    throw new Error(`VirtualMedia '${media}' not found. Available: ${names.join(', ')}`);
  }
  return { odata: match['@odata.id'], names };
}

export async function mountIso(ctx: Ctx, isoUrl: string, media: string): Promise<void> {
  const { odata } = await findMedia(ctx, media);
  const detail = odata.startsWith('http') ? odata : ctx.base + odata;
  const res = await rfFetch(ctx, `${detail.replace(/\/+$/, '')}/VirtualMedia.InsertMedia`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      Image: isoUrl,
      TransferMethod: 'Download',
      TransferProtocolType: 'HTTP',
    }),
  });
  await requireOk(res, 'mount ISO');
}

export async function unmountIso(ctx: Ctx, media: string): Promise<void> {
  const { odata } = await findMedia(ctx, media);
  const detail = odata.startsWith('http') ? odata : ctx.base + odata;
  const res = await rfFetch(ctx, `${detail.replace(/\/+$/, '')}/VirtualMedia.EjectMedia`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
  });
  await requireOk(res, 'unmount ISO');
}
