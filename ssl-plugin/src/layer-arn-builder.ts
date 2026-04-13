export const DEFAULT_LAYER_ACCOUNT_ID = '115813213817';

export function buildLayerArn(
  region: string,
  layerName: string,
  layerVersion: number,
  accountId?: string,
  layerNamePrefix?: string,
): string {
  const account = accountId || DEFAULT_LAYER_ACCOUNT_ID;
  const prefixedName = layerNamePrefix ? `${layerNamePrefix}${layerName}` : layerName;
  return `arn:aws:lambda:${region}:${account}:layer:${prefixedName}:${layerVersion}`;
}
