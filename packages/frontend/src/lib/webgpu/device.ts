/**
 * WebGPU device factory. The device is created once and shared across all
 * GPU-backed components.
 *
 * The factory does not retain the device — callers cache the result they
 * receive so that subsequent calls do not request a new device.
 */

export interface WebGpuContext {
  readonly device: GPUDevice;
  readonly adapter: GPUAdapter;
  readonly format: GPUTextureFormat;
}

export async function createWebGpuContext(canvas: HTMLCanvasElement): Promise<WebGpuContext> {
  const gpu = (navigator as Navigator & { gpu?: GPU }).gpu;
  if (!gpu) {
    throw new Error('WebGPU is not available');
  }
  const adapter = await gpu.requestAdapter();
  if (!adapter) {
    throw new Error('No GPU adapter available');
  }
  const device = await adapter.requestDevice();
  device.lost.catch((err: unknown) => {
    // Surface device loss to the console. The caller decides how to recover.
    console.error('WebGPU device lost', err);
  });

  const context = canvas.getContext('webgpu');
  if (!context) {
    throw new Error('Failed to acquire WebGPU context from canvas');
  }
  const format = gpu.getPreferredCanvasFormat();
  context.configure({ device, format, alphaMode: 'opaque' });

  return { device, adapter, format };
}
