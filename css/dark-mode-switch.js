(function() {
  var darkSwitch = document.getElementById("darkSwitch");
  if (darkSwitch) {
    initTheme();
    darkSwitch.addEventListener("change", function(event) {
      resetTheme();
    });
    function enableDark() {
      document.body.setAttribute("data-theme", "dark");
      document.querySelectorAll("table").forEach(
        function(table) {
          table.classList.add("table-dark");
        }
      );
    }
    function disableDark() {
      document.body.removeAttribute("data-theme");
      document.querySelectorAll("table").forEach(
        function(table) {
          table.classList.remove("table-dark");
        }
      );
    }
    function initTheme() {
      var darkThemeSelected = localStorage.getItem("darkSwitch") !== null
        ? localStorage.getItem("darkSwitch") === "dark"
        : window.matchMedia("(prefers-color-scheme: dark)").matches;
      darkSwitch.checked = darkThemeSelected;
      darkThemeSelected
        ? enableDark()
        : disableDark();
      if (darkThemeSelected) {
        window.addEventListener('DOMContentLoaded', enableDark, false);
      }
    }
    function resetTheme() {
      if (darkSwitch.checked) {
        enableDark();
        localStorage.setItem("darkSwitch", "dark");
      } else {
        disableDark();
        localStorage.setItem("darkSwitch", "light");
      }
    }
  }
})();
