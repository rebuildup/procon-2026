/**
 * WebGPU support detection. Runs once per page load.
 *
 * The browser globals (`navigator.gpu`, `GPUAdapter`) are not available in
 * every runtime (Node, jsdom, …), so we feature-detect defensively.
 */

export type WebGpuSupport =
  { readonly available: true } | { readonly available: false; readonly reason: string };

export async function detectWebGpuSupport(): Promise<WebGpuSupport> {
  if (typeof navigator === 'undefined' || !('gpu' in navigator)) {
    return { available: false, reason: 'navigator.gpu is not exposed' };
  }
  const gpu = (navigator as Navigator & { gpu?: GPU }).gpu;
  if (!gpu) {
    return { available: false, reason: 'WebGPU API disabled' };
  }
  try {
    const adapter = await gpu.requestAdapter();
    if (!adapter) {
      return { available: false, reason: 'No GPU adapter available' };
    }
    return { available: true };
  } catch (err) {
    return {
      available: false,
      reason: err instanceof Error ? err.message : 'adapter request failed',
    };
  }
}
