import FactorioDialogDelete, { FactorioDialogDeleteApi } from "@/components/FactorioDialogDelete";
import { Portal } from "solid-js/web";

export default function ResetAllDataButton() {
  let refDialogReset!: FactorioDialogDeleteApi;

  return (
    <>
      <button class="button button-red icon-button w-full h-8 self-start" type="button" onClick={() => refDialogReset.show()}>
        Reset all data
      </button>

      <Portal>
        <FactorioDialogDelete
          ref={(api) => (refDialogReset = api)}
          onConfirm={() => {
            Promise.all([
              (async () => {
                const registrations = await navigator.serviceWorker.getRegistrations();

                await Promise.all(registrations.map((registration) => registration.unregister()));

                console.log(`Unregistered ${registrations.length} service workers.`);
              })(),
              indexedDB.databases().then((dbs) => Promise.all(dbs.map(({ name }) => name && indexedDB.deleteDatabase(name)))),
            ]).then(() => {
              localStorage.clear();
              window.location.reload();
            });
          }}
        >
          <h2>Are you sure you want to delete all data?</h2>
          <div class="text-white-500 text-center">
            This will remove all signal presets, configuration data, and reset all options back to their default values.
          </div>
        </FactorioDialogDelete>
      </Portal>
    </>
  );
}
