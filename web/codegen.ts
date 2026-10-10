import type { CodegenConfig } from "@graphql-codegen/cli";

// Erzeugt TypeScript-Typen aus dem exportierten Schema (`web/schema.graphql`, siehe scripts/check.sh).
// Ausgabe `src/gql/` ist nicht eingecheckt; `npm run typecheck` ruft den Generator auf.
const config: CodegenConfig = {
  schema: "./schema.graphql",
  generates: {
    "./src/gql/graphql.ts": {
      plugins: ["typescript"],
      config: {
        scalars: { DateTime: "string", Date: "string", UUID: "string", JSON: "unknown" },
        useTypeImports: true,
        enumsAsTypes: true,
        avoidOptionals: { field: true },
        maybeValue: "T | null",
      },
    },
  },
  hooks: { afterAllFileWrite: [] },
};

export default config;
