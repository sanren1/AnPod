// Launch parameters the PC passes through `hdc shell aa start --ps` (they arrive as strings).
export interface LaunchParams {
  port: number;
  hostId: string;
  launchToken: string;
}

const HEX32 = /^[0-9a-f]{32}$/;

function parsePort(value: Object | undefined): number | undefined {
  const text = typeof value === 'number' ? String(value) : value;
  if (typeof text !== 'string' || !/^[0-9]{5}$/.test(text)) {
    return undefined;
  }
  const port = Number(text);
  return port >= 40000 && port <= 49999 ? port : undefined;
}

// undefined unless port is 40000-49999 and hostId / launchToken are 32 lowercase hex digits.
export function parseLaunchParams(parameters: Record<string, Object> | undefined): LaunchParams | undefined {
  if (parameters === undefined) {
    return undefined;
  }
  const port = parsePort(parameters['port']);
  const hostId = parameters['hostId'];
  const launchToken = parameters['launchToken'];
  if (port === undefined || typeof hostId !== 'string' || typeof launchToken !== 'string' ||
    !HEX32.test(hostId) || !HEX32.test(launchToken)) {
    return undefined;
  }
  return { port: port, hostId: hostId, launchToken: launchToken };
}
