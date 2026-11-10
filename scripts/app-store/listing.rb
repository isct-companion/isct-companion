require "json"

module AppStoreListing
  def self.prepare(app, version, root)
    metadata = JSON.parse(File.read(File.join(root, "apps/ios/appstore/metadata.json")))
    unless app.content_rights_declaration
      app.update(attributes: { contentRightsDeclaration: metadata.fetch("contentRightsDeclaration") })
    end
    review = version.fetch_app_store_review_detail
    raise "App Review contact information must be completed first" unless review
    review.update(attributes: {
      demoAccountRequired: metadata.fetch("demoAccountRequired"), notes: metadata.fetch("reviewNotes")
    })
    initialize_free_price(app)
  end

  def self.initialize_free_price(app)
    # fastlane's price_tier action uses the retired inline-price relationship.
    # Reuse its authenticated client for Apple's current price-schedule API.
    client = Spaceship::ConnectAPI.client.tunes_request_client
    begin
      schedule = client.get("v1/apps/#{app.id}/appPriceSchedule", { include: "manualPrices" }).body
      return if (schedule["included"] || []).any? { |item| item["type"] == "appPrices" }
    rescue Spaceship::UnexpectedResponse => error
      raise unless error.message.match?(/404|NOT_FOUND|does not exist/)
    end
    page = client.get("v1/apps/#{app.id}/appPricePoints", { "filter[territory]" => "JPN", limit: 200 })
    free = nil
    loop do
      free = page.body.fetch("data").find { |point| point.dig("attributes", "customerPrice").to_s.match?(/\A0(?:\.0+)?\z/) }
      break if free || !page.next_url
      page = client.get(page.next_url)
    end
    raise "Apple returned no free price point for Japan" unless free
    price_id = "${isct-free-price}"
    client.post("v1/appPriceSchedules", {
      data: { type: "appPriceSchedules", relationships: {
        app: { data: { type: "apps", id: app.id } },
        baseTerritory: { data: { type: "territories", id: "JPN" } },
        manualPrices: { data: [{ type: "appPrices", id: price_id }] }
      } },
      included: [{ type: "appPrices", id: price_id, attributes: { startDate: nil, endDate: nil },
        relationships: { appPricePoint: { data: { type: "appPricePoints", id: free.fetch("id") } } } }]
    })
    puts "Initialized free App Store pricing"
  end
end
