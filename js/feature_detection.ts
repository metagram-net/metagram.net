document.addEventListener("DOMContentLoaded", () => {
  document.body.classList.add("js");

  document.querySelectorAll(".js-show").forEach((elt) => {
    elt.classList.remove("js-show");
  });

  document.querySelectorAll(".js-hidden").forEach((elt) => {
    elt.classList.add("hidden");
    elt.classList.remove("js-hidden");
  });
});
