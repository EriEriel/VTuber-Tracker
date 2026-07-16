// Monkey patch Bun's missing node:v8 startupSnapshot.isBuildingSnapshot API
const originalGetBuiltinModule = (process as any).getBuiltinModule;
if (originalGetBuiltinModule) {
  (process as any).getBuiltinModule = function (name: string) {
    if (name === 'v8') {
      try {
        const mod = originalGetBuiltinModule.call(process, 'v8');
        if (mod) {
          // If startupSnapshot doesn't exist or is read-only, we can override the whole object or properties
          const originalSnapshot = mod.startupSnapshot || {};
          const mockSnapshot = Object.create(originalSnapshot);
          mockSnapshot.isBuildingSnapshot = () => false;
          
          Object.defineProperty(mod, 'startupSnapshot', {
            value: mockSnapshot,
            writable: true,
            configurable: true,
          });
        }
        return mod;
      } catch (e) {
        console.error('Polyfill error:', e);
      }
    }
    return originalGetBuiltinModule.apply(this, arguments);
  };
}
