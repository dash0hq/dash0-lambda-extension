import * as dash0 from "./distro/dist/src/distro.js";
import {register} from "module";

try {

    await dash0.init;

// `register` needs `import.meta.url`, which is why this call cannot move into the distro
// along with the instrumentations: `distro/tsconfig.json` compiles to CommonJS, where
// `import.meta` does not exist.
    register('import-in-the-middle/hook.mjs', import.meta.url);

} catch (err) {
    console.error('Error initializing Dash0 tracer:', err);
}
