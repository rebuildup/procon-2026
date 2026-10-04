import { afterEach, describe, expect, it, mock } from 'bun:test';
import { createWebGpuContext } from './device';

const originalGpu = Object.getOwnPropertyDescriptor(navigator, 'gpu');
afterEach(() => {
  if (originalGpu) Object.defineProperty(navigator, 'gpu', originalGpu);
  else Reflect.deleteProperty(navigator, 'gpu');
});

function setup() {
  const destroy = mock(() => {});
  const device = { destroy, lost: new Promise(() => {}) } as unknown as GPUDevice;
  const configure = mock(() => {});
  const context = { configure } as unknown as GPUCanvasContext;
  const getContext = mock((): GPUCanvasContext | null => context);
  const canvas = { getContext } as unknown as HTMLCanvasElement;
  const requestDevice = mock(() => Promise.resolve(device));
  const adapter = { requestDevice } as unknown as GPUAdapter;
  const gpu = {
    requestAdapter: mock(() => Promise.resolve(adapter)),
    getPreferredCanvasFormat: mock(() => 'bgra8unorm' as GPUTextureFormat),
  };
  Object.defineProperty(navigator, 'gpu', { value: gpu, configurable: true });
  return { destroy, device, configure, getContext, canvas, requestDevice, adapter, gpu };
}

describe('WebGPU context ownership', () => {
  it('releases the device when the canvas has no WebGPU context', async () => {
    const state = setup();
    state.getContext.mockReturnValue(null);
    await expect(createWebGpuContext(state.canvas)).rejects.toThrow('WebGPU context');
    expect(state.destroy).toHaveBeenCalledTimes(1);
  });

  it('releases the device when canvas configuration throws', async () => {
    const state = setup();
    const error = new Error('configuration failed');
    state.configure.mockImplementation(() => {
      throw error;
    });
    await expect(createWebGpuContext(state.canvas)).rejects.toBe(error);
    expect(state.destroy).toHaveBeenCalledTimes(1);
  });

  it('releases the device when preferred format lookup throws', async () => {
    const state = setup();
    const error = new Error('format failed');
    state.gpu.getPreferredCanvasFormat.mockImplementation(() => {
      throw error;
    });
    await expect(createWebGpuContext(state.canvas)).rejects.toBe(error);
    expect(state.destroy).toHaveBeenCalledTimes(1);
  });

  it('transfers a successfully configured device to the caller', async () => {
    const state = setup();
    expect(await createWebGpuContext(state.canvas)).toEqual({
      device: state.device,
      adapter: state.adapter,
      format: 'bgra8unorm',
    });
    expect(state.destroy).not.toHaveBeenCalled();
    expect(state.configure).toHaveBeenCalledWith({
      device: state.device,
      format: 'bgra8unorm',
      alphaMode: 'opaque',
    });
  });

  it('preserves a requestDevice failure before device ownership exists', async () => {
    const state = setup();
    const error = new Error('request failed');
    state.requestDevice.mockRejectedValue(error);
    await expect(createWebGpuContext(state.canvas)).rejects.toBe(error);
    expect(state.destroy).not.toHaveBeenCalled();
    expect(state.getContext).not.toHaveBeenCalled();
  });
});
