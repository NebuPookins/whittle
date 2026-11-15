import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

const EFFECT_MESSAGE =
  "React effects must use a block body: `useEffect(() => { f(); }, deps)`. " +
  "The callback's return value is treated as the cleanup function, and DOM methods typed `void` " +
  "can return non-functions at runtime (e.g. `scrollIntoView` returns a Promise in newer browsers), " +
  "which crashes React on unmount.";

export default tseslint.config(
  { ignores: ["dist", "src-tauri", "node_modules"] },
  js.configs.recommended,
  tseslint.configs.recommended,
  reactHooks.configs.flat.recommended,
  {
    languageOptions: { globals: globals.browser },
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector:
            "CallExpression[callee.name=/^use(Layout)?Effect$/] > ArrowFunctionExpression.arguments:first-child[expression=true]",
          message: EFFECT_MESSAGE,
        },
        {
          selector:
            "CallExpression[callee.property.name=/^use(Layout)?Effect$/] > ArrowFunctionExpression.arguments:first-child[expression=true]",
          message: EFFECT_MESSAGE,
        },
      ],
    },
  },
);
