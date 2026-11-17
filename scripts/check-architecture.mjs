// Resolve aliases with the same TypeScript configuration as the production build.
import path from 'node:path';
import ts from 'typescript';

const root = process.cwd();
const config = ts.readConfigFile('tsconfig.app.json', ts.sys.readFile);
if (config.error) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, '\n'));
const { fileNames, options } = ts.parseJsonConfigFileContent(config.config, ts.sys, root);
const production = fileNames.filter((name) => !/\.test\.tsx?$|\.d\.ts$/.test(name));
const modules = new Set(production);
const graph = new Map();
const problems = new Set();
const local = (file) => path.relative(path.join(root, 'src'), file).split(path.sep).join('/');

function inspect(file, specifier, edges) {
  const source = local(file);
  if (specifier.startsWith('@tauri-apps/') && !source.startsWith('platform/')) {
    problems.add(`${source} must access the native SDK through platform/`);
  }
  const target = ts.resolveModuleName(specifier, file, options, ts.sys).resolvedModule?.resolvedFileName;
  if (!modules.has(target)) return;
  edges.add(target);
  const dest = local(target);
  const [layer, feature] = source.split('/');
  const [targetLayer, targetFeature] = dest.split('/');
  if (
    (layer === 'contracts' && targetLayer !== 'contracts') ||
    (['shared', 'platform'].includes(layer) && ['app', 'features'].includes(targetLayer)) ||
    (layer === 'features' && targetLayer === 'app') ||
    (layer === 'features' &&
      targetLayer === 'features' &&
      feature !== targetFeature &&
      /Page\.tsx$/.test(dest))
  ) {
    problems.add(`Forbidden dependency: ${source} -> ${dest}`);
  }
}

for (const file of production) {
  const tree = ts.createSourceFile(file, ts.sys.readFile(file), ts.ScriptTarget.Latest, true);
  const edges = new Set();
  function visit(node) {
    let specifier;
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) specifier = node.moduleSpecifier;
    if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) specifier = node.argument.literal;
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword)
      specifier = node.arguments[0];
    if (specifier && ts.isStringLiteralLike(specifier)) inspect(file, specifier.text, edges);
    ts.forEachChild(node, visit);
  }
  visit(tree);
  graph.set(file, edges);
}

const finished = new Set();
const active = new Set();
function walk(file, ancestors) {
  if (active.has(file)) {
    problems.add(
      `Import cycle: ${[...ancestors.slice(ancestors.indexOf(file)), file].map(local).join(' -> ')}`,
    );
    return;
  }
  if (finished.has(file)) return;
  active.add(file);
  for (const next of graph.get(file)) walk(next, [...ancestors, file]);
  active.delete(file);
  finished.add(file);
}
for (const file of production) walk(file, []);
if (problems.size) throw new Error([...problems].join('\n'));
