import { type Component, For } from "solid-js";
import { RECIPES } from "../lib/formats";
import { chooseFiles } from "../lib/store";

/** Recipe cards: pick one, then pick files, and the editor opens preset. */
export const RecipeGrid: Component<{ large?: boolean }> = (props) => (
  <div class="recipes" classList={{ large: props.large }}>
    <For each={RECIPES}>
      {(recipe, index) => (
        <button
          type="button"
          class="recipe"
          data-tone={recipe.tone}
          style={{ "--i": index() }}
          onClick={() => void chooseFiles(recipe.id)}
        >
          <span class="recipe-title">{recipe.title}</span>
          <span class="recipe-detail">{recipe.detail}</span>
          <span class="recipe-route">{recipe.route}</span>
        </button>
      )}
    </For>
  </div>
);
