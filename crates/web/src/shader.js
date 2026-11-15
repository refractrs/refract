// WebGPU presentation for dedicated shader views. The Rust host owns scheduling;
// asynchronous initialization always draws the latest committed frame.
let shared = null;
const surfaces = new WeakMap();

// cloneNode does not copy a canvas bitmap. Freeze shader pixels into the exit
// snapshot before the original surface and its GPU resources are released.
export function copy_shader_canvases(original, clone) {
  const sources = original.querySelectorAll('canvas');
  const copies = clone.querySelectorAll('canvas');
  sources.forEach((source, index) => {
    const surface = surfaces.get(source);
    if (!surface || !surface.pipeline || surface.failed) return;
    const target = copies[index];
    surface.render();
    target.getContext('2d').drawImage(source, 0, 0);
  });
}

function acquire(surface) {
  if (!shared) {
    const runtime = { surfaces: new Set(), pipelines: new Map(), device: null };
    shared = runtime;
    runtime.ready = (async () => {
      if (!globalThis.navigator?.gpu) throw new Error('WebGPU is unavailable');
      const adapter = await navigator.gpu.requestAdapter();
      if (!adapter) throw new Error('No WebGPU adapter is available');
      const device = await adapter.requestDevice();
      runtime.device = device;
      runtime.format = navigator.gpu.getPreferredCanvasFormat();
      if (!runtime.surfaces.size) device.destroy();
      device.lost.then(info => {
        if (shared === runtime) shared = null;
        for (const view of runtime.surfaces) view.fail(`WebGPU device lost: ${info.message}`);
        runtime.pipelines.clear();
      });
      device.addEventListener('uncapturederror', event => {
        for (const view of runtime.surfaces) view.fail(event.error.message);
      });
      return runtime;
    })();
  }
  shared.surfaces.add(surface);
  return shared;
}

async function pipeline(runtime, source) {
  let promise = runtime.pipelines.get(source);
  if (!promise) {
    promise = (async () => {
      const device = runtime.device;
      const module = device.createShaderModule({ label: 'Refract application shader', code: source });
      const info = await module.getCompilationInfo();
      const errors = info.messages.filter(message => message.type === 'error');
      if (errors.length) throw new Error(errors.map(e => `${e.lineNum}:${e.linePos}: ${e.message}`).join('\n'));
      const bindings = device.createBindGroupLayout({ entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: 'uniform' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, buffer: { type: 'uniform' } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
        { binding: 3, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
      ] });
      return device.createRenderPipelineAsync({
        label: 'Refract application shader',
        layout: device.createPipelineLayout({ bindGroupLayouts: [bindings] }),
        vertex: { module, entryPoint: 'refract_vertex' },
        fragment: { module, entryPoint: 'refract_fragment_premultiplied', targets: [{ format: runtime.format }] },
        primitive: { topology: 'triangle-list' },
      });
    })();
    runtime.pipelines.set(source, promise);
    // Bound retained programs even when an editor continuously supplies new source.
    if (runtime.pipelines.size > 32) runtime.pipelines.delete(runtime.pipelines.keys().next().value);
  }
  return promise;
}

export class ShaderSurface {
  constructor(host) {
    this.host = host;
    this.container = host.ownerDocument.createElement('div');
    this.container.style.cssText = 'position:absolute;pointer-events:none;overflow:hidden';
    this.container.setAttribute('aria-hidden', 'true');
    this.canvas = host.ownerDocument.createElement('canvas');
    this.canvas.style.cssText = 'display:none;width:100%;height:100%';
    this.container.append(this.canvas);
    surfaces.set(this.canvas, this);
    // Replace the deterministic prerendered SVG fallback once the host takes over.
    host.replaceChildren(this.container);
    this.version = 0;
    this.disposed = false;
    this.failed = false;
    this.source = null;
    this.image = new Uint8Array([0, 0, 0, 0]);
    this.imageWidth = this.imageHeight = 1;
    this.imageDirty = true;
    host.dataset.refractShader = 'pending';
    this.runtime = acquire(this);
    this.runtime.ready.then(() => {
      if (!this.disposed && !this.failed) this.prepare();
    }).catch(error => this.fail(error.message));
  }

  set_program(source) {
    if (source === this.source || this.disposed) return;
    this.source = source;
    this.version++;
    this.pipeline = null;
    this.bindings = null;
    if (this.runtime.device && !this.failed) this.prepare();
  }

  set_image(pixels, width, height) {
    if (this.disposed) return;
    this.image = pixels.length ? pixels.slice() : new Uint8Array([0, 0, 0, 0]);
    this.imageWidth = width;
    this.imageHeight = height;
    this.imageDirty = true;
  }

  async prepare() {
    if (!this.source) return;
    const version = this.version;
    try {
      const result = await pipeline(this.runtime, this.source);
      if (this.disposed || this.failed || version !== this.version) return;
      this.pipeline = result;
      this.bindings = null;
      if (!this.context) {
        this.context = this.canvas.getContext('webgpu');
        if (!this.context) throw new Error('Cannot create a WebGPU canvas context');
        this.context.configure({ device: this.runtime.device, format: this.runtime.format, alphaMode: 'premultiplied' });
      }
      this.render();
    } catch (error) {
      if (version === this.version) this.fail(error.message);
    }
  }

  update(uniforms, frame, width, height, left, top, fallback) {
    if (this.disposed) return;
    // wasm-bindgen slices are borrowed from WASM memory. Retain owned copies.
    this.fallback = fallback;
    this.latest = { uniforms: uniforms.slice(), frame: frame.slice(), width, height };
    const data = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
    const style = `position:absolute;pointer-events:none;overflow:hidden;left:${left}px;top:${top}px;width:${data.getFloat32(0, true)}px;height:${data.getFloat32(4, true)}px;background:${this.pipeline && !this.failed ? 'transparent' : fallback}`;
    if (this.style !== style) { this.container.style.cssText = style; this.style = style; }
    this.render();
  }

  render() {
    if (this.disposed || this.failed || !this.pipeline || !this.latest) return;
    try {
      const device = this.runtime.device;
      const latest = this.latest;
      const width = Math.max(1, Math.min(latest.width, device.limits.maxTextureDimension2D, 8192));
      const height = Math.max(1, Math.min(latest.height, device.limits.maxTextureDimension2D, 8192));
      if (this.canvas.width !== width) this.canvas.width = width;
      if (this.canvas.height !== height) this.canvas.height = height;
      const frame = latest.frame.slice();
      const view = new DataView(frame.buffer);
      view.setFloat32(8, width, true);
      view.setFloat32(12, height, true);
      if (!this.frameBuffer) this.frameBuffer = device.createBuffer({ size: 32, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
      if (!this.uniformBuffer || this.uniformBuffer.size !== latest.uniforms.length) {
        this.uniformBuffer?.destroy();
        this.uniformBuffer = device.createBuffer({ size: latest.uniforms.length, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
        this.bindings = null;
      }
      if (this.imageDirty) {
        if (!this.input || this.input.width !== this.imageWidth || this.input.height !== this.imageHeight) {
          this.input?.destroy();
          this.input = device.createTexture({
            size: [this.imageWidth, this.imageHeight], format: 'rgba8unorm',
            usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
          });
          this.bindings = null;
        }
        device.queue.writeTexture({ texture: this.input }, this.image,
          { bytesPerRow: this.imageWidth * 4, rowsPerImage: this.imageHeight }, [this.imageWidth, this.imageHeight]);
        this.imageDirty = false;
      }
      if (!this.bindings) {
        this.sampler ??= device.createSampler({ magFilter: 'linear', minFilter: 'linear' });
        this.bindings = device.createBindGroup({ layout: this.pipeline.getBindGroupLayout(0), entries: [
          { binding: 0, resource: { buffer: this.frameBuffer } },
          { binding: 1, resource: { buffer: this.uniformBuffer } },
          { binding: 2, resource: this.input.createView() },
          { binding: 3, resource: this.sampler },
        ] });
      }
      device.queue.writeBuffer(this.frameBuffer, 0, frame);
      device.queue.writeBuffer(this.uniformBuffer, 0, latest.uniforms);
      const encoder = device.createCommandEncoder({ label: 'Refract shader frame' });
      const pass = encoder.beginRenderPass({ colorAttachments: [{
        view: this.context.getCurrentTexture().createView(),
        clearValue: [0, 0, 0, 0], loadOp: 'clear', storeOp: 'store',
      }] });
      pass.setPipeline(this.pipeline);
      pass.setBindGroup(0, this.bindings);
      pass.draw(3);
      pass.end();
      device.queue.submit([encoder.finish()]);
      if (this.canvas.style.display !== 'block') this.canvas.style.display = 'block';
      // The fallback must not show through transparent shader pixels.
      this.container.style.background = 'transparent';
      if (this.host.dataset.refractShader !== 'ready') this.host.dataset.refractShader = 'ready';
    } catch (error) { this.fail(error.message); }
  }

  active() { return !this.disposed && !this.failed; }

  fail(message) {
    if (this.disposed || this.failed) return;
    this.failed = true;
    this.canvas.style.display = 'none';
    this.host.dataset.refractShader = 'unavailable';
    this.host.dataset.refractShaderError = message;
    // Preserve the fallback through device loss as well as initial adapter failure.
    if (this.fallback) this.container.style.background = this.fallback;
    console.warn(`Refract shader: ${message}`);
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.version++;
    this.context?.unconfigure();
    this.frameBuffer?.destroy();
    this.uniformBuffer?.destroy();
    this.input?.destroy();
    this.container.remove();
    surfaces.delete(this.canvas);
    this.runtime.surfaces.delete(this);
    if (!this.runtime.surfaces.size) {
      this.runtime.pipelines.clear();
      this.runtime.device?.destroy();
      if (shared === this.runtime) shared = null;
    }
    this.latest = this.pipeline = this.bindings = this.image = null;
  }
}
