import { SettingsPage } from "../components/SettingsPage";
import { openExternal } from "../settings/api";
import { useSettings } from "../settings/SettingsContext";
import "./AboutPage.css";

const PROJECT_URL = "https://github.com/eqlaika/stonemite";
const EMAIL = "laika@laikasoft.co";

export function AboutPage() {
  const { runtime } = useSettings();

  return (
    <SettingsPage
      title="About Stonemite"
      description="Version, project, author, and contact information."
    >
      <section className="about-product" aria-labelledby="about-product-name">
        <img
          className="about-product-logo"
          src="/app.png"
          width="72"
          height="72"
          alt="Stonemite logo"
        />
        <div className="about-product-copy">
          <h2 id="about-product-name">Stonemite</h2>
          <p className="about-version">
            Version <span>{runtime?.version ?? "unavailable"}</span>
          </p>
          <p className="about-description">EverQuest multiboxing tool</p>
        </div>
      </section>

      <section className="about-section" aria-labelledby="about-project-title">
        <h2 id="about-project-title">Project</h2>
        <p>
          Source code, releases, and issue tracking are available on GitHub.
        </p>
        <a
          className="about-link"
          href={PROJECT_URL}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event) => {
            event.preventDefault();
            void openExternal(PROJECT_URL);
          }}
        >
          github.com/eqlaika/stonemite
        </a>
      </section>

      <section className="about-author" aria-labelledby="about-author-name">
        <img
          className="about-author-image"
          src="/author.png"
          width="72"
          height="72"
          alt="Laika, author of Stonemite"
        />
        <div>
          <h2 id="about-author-name">Laika</h2>
          <p>Author and maintainer of Stonemite.</p>
        </div>
      </section>

      <section className="about-section" aria-labelledby="about-contact-title">
        <h2 id="about-contact-title">Contact</h2>
        <address>
          <dl className="about-contact">
            <div>
              <dt>In game</dt>
              <dd>
                <code className="about-contact-value">/tell Xegony.Laika</code>
              </dd>
            </div>
            <div>
              <dt>Email</dt>
              <dd>
                <a
                  className="about-link about-contact-value"
                  href={`mailto:${EMAIL}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  onClick={(event) => {
                    event.preventDefault();
                    void openExternal(`mailto:${EMAIL}`);
                  }}
                >
                  {EMAIL}
                </a>
              </dd>
            </div>
          </dl>
        </address>
      </section>
    </SettingsPage>
  );
}
