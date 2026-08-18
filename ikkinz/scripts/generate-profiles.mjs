import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { strToU8, zipSync } from "fflate";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pluginRoot = path.join(root, "co.laikasoft.ikkinz.sdPlugin");
const pluginManifest = JSON.parse(
  await readFile(path.join(pluginRoot, "manifest.json"), "utf8"),
);
const profileName = "Stonemite · EQ boxing";
const plugin = {
  Name: pluginManifest.Name,
  UUID: pluginManifest.UUID,
  Version: pluginManifest.Version,
};
const actionsByUuid = new Map(
  pluginManifest.Actions.map((action) => [action.UUID, action]),
);

const layout = {
  "0,0": "character-slot-1",
  "1,0": "character-slot-2",
  "2,0": "character-slot-3",
  "3,0": "group",
  "4,0": "broadcast",
  "0,1": "character-slot-4",
  "1,1": "character-slot-5",
  "2,1": "character-slot-6",
  "3,1": "follow",
  "0,2": "logo",
  "3,2": "assist",
  "4,2": "swap",
};

const profiles = [
  {
    filename: "Ikkinz.streamDeckProfile",
    deviceModel: "20GBL9901",
    deviceUuid: "50201e4b-d69d-4824-b0fa-44e12da79d25",
    rootId: "7287AD4A-9A86-4CA4-A686-A438839F6254",
    dashboardId: "7704B1EF-5317-45D8-B60A-968BF0965494",
    emptyId: "DC08B3AA-FB89-40F7-BE58-721C70FD2AC8",
    actionIds: {
      "0,0": "c2fe0a07-528f-58bd-999e-c88e1b497d82",
      "1,0": "279dbfce-d8a6-51d2-869c-7c947977159b",
      "2,0": "dcc044c8-e68b-5e0e-a241-fa871d858f99",
      "3,0": "188e35bb-b7f0-5e32-8b8d-818c0013e99c",
      "4,0": "937f2b40-732a-565b-83de-6064cf5c2082",
      "0,1": "1e2a92cc-b4f2-59b1-85fa-eb5000fb8f4f",
      "1,1": "fa5ff810-b102-5890-95fb-0f634c2bc04e",
      "2,1": "93cc25d4-4eef-51b6-a211-df52a7fd268b",
      "3,1": "0b6fca00-2a68-5815-8540-56ce374be6e2",
      "0,2": "f08a466d-6c92-566c-b4f0-5edb1a55775f",
      "3,2": "96ff4400-5437-53ac-bef0-1501ca9882a8",
      "4,2": "44430f44-e46b-534a-b63b-9f96f47f3692",
    },
  },
  {
    filename: "Ikkinz Mobile.streamDeckProfile",
    deviceModel: "VSD2/WiFi",
    deviceUuid: "e0563b90-7747-4d14-b5b8-390bd03c6499",
    rootId: "7D8E24CF-07E7-4C57-B938-3239F2E0C37A",
    dashboardId: "F40EBB47-A0BA-4B41-9B22-EA9A3AF6A953",
    emptyId: "6C2D4E89-2FB5-4E9A-A662-52122086AD6B",
    actionIds: {
      "0,0": "6094091f-6bb0-50ad-bf6e-64474c72881a",
      "1,0": "5a423392-9558-5989-99a5-935b2a254ff9",
      "2,0": "370cdd0d-1253-579e-aa8e-c8101fe830a7",
      "3,0": "a9599fd4-a3f0-5820-aabb-e2123c71a72e",
      "4,0": "b41b3948-1812-5694-a561-205b795d3ba3",
      "0,1": "3601e300-d5df-5c2e-ab9d-689b4c3290d3",
      "1,1": "fd0f24f8-8ce8-5eab-8666-d7072738be78",
      "2,1": "66ddcd37-0657-59eb-9b48-b6b95ff3ca79",
      "3,1": "9b05fa7c-8ced-5b24-8480-3d2a93f93b3e",
      "0,2": "f8cfa3cb-2eb2-5a68-b4ac-8f0103679d74",
      "3,2": "7950bf24-8a1f-58be-9193-2a6cad852bb2",
      "4,2": "5acf5c80-3a5e-55d4-a962-565edb58ea84",
    },
  },
];

for (const profile of profiles) {
  const rootPath = `Profiles/${profile.rootId}.sdProfile`;
  const dashboardPath = `${rootPath}/Profiles/${profile.dashboardId}`;
  const emptyPath = `${rootPath}/Profiles/${profile.emptyId}`;
  const actions = Object.fromEntries(
    Object.entries(layout).map(([position, actionName]) => {
      const uuid = `${plugin.UUID}.${actionName}`;
      const definition = actionsByUuid.get(uuid);
      if (!definition) throw new Error(`Missing manifest action ${uuid}.`);
      const actionId = profile.actionIds[position];
      if (!actionId)
        throw new Error(
          `Missing ${profile.filename} action ID at ${position}.`,
        );
      return [
        position,
        {
          ActionID: actionId,
          LinkedTitle: true,
          Name: definition.Name,
          Plugin: plugin,
          Resources: null,
          Settings: {},
          State: 0,
          States: [
            {
              FontFamily: "",
              FontSize: 12,
              FontStyle: "",
              FontUnderline: false,
              OutlineThickness: 2,
              ShowTitle: false,
              TitleAlignment: "bottom",
              TitleColor: "#ffffff",
            },
          ],
          UUID: uuid,
        },
      ];
    }),
  );

  const packageManifest = {
    AppVersion: "7.5.1.22901",
    DeviceModel: profile.deviceModel,
    DeviceSettings: null,
    FormatVersion: 1,
    OSType: "macOS",
    OSVersion: "26.5.2",
    RequiredPlugins: [plugin.UUID],
  };
  const rootManifest = {
    Device: { Model: profile.deviceModel, UUID: profile.deviceUuid },
    Name: profileName,
    Pages: {
      Current: "00000000-0000-0000-0000-000000000000",
      Default: profile.emptyId.toLowerCase(),
      Pages: [profile.dashboardId.toLowerCase()],
    },
    Version: "3.0",
  };
  const dashboardManifest = {
    Controllers: [{ Actions: actions, Type: "Keypad" }],
    Icon: "",
    Name: profileName,
  };
  const emptyManifest = {
    Controllers: [{ Actions: null, Type: "Keypad" }],
    Icon: "",
    Name: "",
  };
  const json = (value) => strToU8(JSON.stringify(value));
  const archive = zipSync(
    {
      "package.json": json(packageManifest),
      "Profiles/": new Uint8Array(),
      [`${rootPath}/`]: new Uint8Array(),
      [`${rootPath}/Images/`]: new Uint8Array(),
      [`${rootPath}/manifest.json`]: json(rootManifest),
      [`${rootPath}/Profiles/`]: new Uint8Array(),
      [`${dashboardPath}/`]: new Uint8Array(),
      [`${dashboardPath}/Images/`]: new Uint8Array(),
      [`${dashboardPath}/manifest.json`]: json(dashboardManifest),
      [`${emptyPath}/`]: new Uint8Array(),
      [`${emptyPath}/Images/`]: new Uint8Array(),
      [`${emptyPath}/manifest.json`]: json(emptyManifest),
    },
    { level: 9, mtime: new Date(2026, 0, 1) },
  );
  await writeFile(path.join(pluginRoot, "profiles", profile.filename), archive);
  console.log(`Generated profiles/${profile.filename}.`);
}
