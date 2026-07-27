// Conservative: whitespace/precision/comment cleanup only. Anything that
// restructures nodes risks mangling the foreignObject HTML that carries
// every multi-line label.
export default {
  multipass: true,
  js2svg: { pretty: false },
  plugins: [
    'cleanupAttrs',
    'removeComments',
    'removeMetadata',
    'removeEmptyAttrs',
    'removeEmptyContainers',
    { name: 'cleanupNumericValues', params: { floatPrecision: 2 } },
    { name: 'convertPathData', params: { floatPrecision: 2 } },
  ],
};
