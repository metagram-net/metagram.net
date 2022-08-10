import $ from "jquery";
import select2 from "select2";
select2(window, $);

import "select2/dist/css/select2.css";

document.addEventListener("DOMContentLoaded", () => {
  const defaultOptions = {
    templateSelection: (state) => {
      if (!state.id) {
        return state.text;
      }

      const $opt = $(state.element);

      const $tag = $("<span></span>");
      $tag.addClass("p-1");
      $tag.css("background-color", $opt.data("color"));
      $tag.text(state.text);
      return $tag;
    },
  };

  $(".select-multiple-no-create").select2(defaultOptions);
  $(".select-multiple").select2({
    ...defaultOptions,
    tags: true,
    createTag: ({ term }) => {
      const text = term.trim();
      if (term === "") {
        return null;
      }

      return {
        text,
        // Keep synced with Drops controller
        id: `_${text}`,
        newTag: true,
      };
    },
  });
});
